use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use std::time::{Duration, Instant};
use futures_util::{stream, StreamExt};
use jni::objects::{
    GlobalRef, JByteArray, JClass, JObject, JObjectArray, JString, JValue,
};
use jni::sys::{jboolean, JNI_FALSE, JNI_TRUE};
use jni::{JNIEnv, JavaVM};
use parking_lot::{Mutex, RwLock};
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static ENGINE: Mutex<Option<Arc<Engine>>> = Mutex::new(None);
static FETCH_CANCEL: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

const FETCH_TIMEOUT_SECS: u64 = 120;
const MAX_FILES: usize = 100;
const MAX_FILE_BYTES: usize = 200 * 1024 * 1024;
const MAX_SYNC_MODELS: usize = 100;
const MAX_ANNOUNCE_MODELS: usize = 200;

fn get_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("Failed to initialize multi-threaded Tokio runtime")
    })
}

struct Engine {
    endpoint: iroh::Endpoint,
    store: iroh_blobs::store::fs::FsStore,
    index: RwLock<SearchIndex>,
    peers: RwLock<HashSet<String>>,
}

#[derive(Default)]
struct SearchIndex {
    kw: HashMap<String, HashSet<iroh_blobs::Hash>>,
    meta: HashMap<iroh_blobs::Hash, String>,
}

impl SearchIndex {
    fn get(&self, hash: &iroh_blobs::Hash) -> Option<String> {
        self.meta.get(hash).cloned()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(CHARS[(b >> 4) as usize] as char);
        s.push(CHARS[(b & 0xF) as usize] as char);
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        out.push((digit(b[i])? << 4) | digit(b[i + 1])?);
        i += 2;
    }
    Some(out)
}

fn index_add(index: &RwLock<SearchIndex>, hash: iroh_blobs::Hash, meta_json: &str) {
    let kws = extract_keywords(meta_json);
    let mut idx = index.write();
    for kw in kws {
        idx.kw.entry(kw).or_default().insert(hash);
    }
    idx.meta.insert(hash, meta_json.to_string());
}

fn kw_words(s: &str, min: usize) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > min)
        .map(|w| w.to_lowercase())
        .collect()
}

fn extract_keywords(meta_json: &str) -> Vec<String> {
    let v: serde_json::Value = match serde_json::from_str(meta_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut kws = Vec::new();
    if let Some(t) = v.get("title").and_then(|x| x.as_str()) {
        kws.push(t.to_lowercase());
        kws.extend(kw_words(t, 2));
    }
    if let Some(d) = v.get("description").and_then(|x| x.as_str()) {
        kws.extend(kw_words(d, 3));
    }
    if let Some(c) = v.get("category").and_then(|x| x.as_str()) {
        kws.push(c.to_lowercase());
    }
    if let Some(tags) = v.get("tags").and_then(|x| x.as_array()) {
        for t in tags {
            if let Some(s) = t.as_str() {
                kws.push(s.to_lowercase());
            }
        }
    }
    if let Some(n) = v
        .get("designer")
        .and_then(|d| d.get("name"))
        .and_then(|x| x.as_str())
    {
        kws.push(n.to_lowercase());
    }
    kws.sort();
    kws.dedup();
    kws
}

struct Reporter {
    jvm: JavaVM,
    callback: GlobalRef,
    last_reported: Mutex<Instant>,
    throttle: Duration,
}

impl Reporter {
    fn new(jvm: JavaVM, callback: GlobalRef, throttle_millis: u64) -> Self {
        Self {
            jvm,
            callback,
            last_reported: Mutex::new(Instant::now() - Duration::from_millis(throttle_millis)),
            throttle: Duration::from_millis(throttle_millis),
        }
    }

    fn progress(
        &self,
        status_code: i32,
        progress_pct: i32,
        downloaded: i64,
        total: i64,
        message: &str,
        force: bool,
    ) {
        if !force {
            let mut last = self.last_reported.lock();
            if last.elapsed() < self.throttle {
                return;
            }
            *last = Instant::now();
        }
        if let Ok(mut env) = self.jvm.attach_current_thread_as_daemon() {
            let msg: JObject = match env.new_string(message) {
                Ok(s) => s.into(),
                Err(_) => return,
            };
            let _ = env.call_method(
                &self.callback,
                "onTransferProgress",
                "(IIJJLjava/lang/String;)V",
                &[
                    JValue::Int(status_code),
                    JValue::Int(progress_pct),
                    JValue::Long(downloaded),
                    JValue::Long(total),
                    JValue::Object(&msg),
                ],
            );
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
        }
    }

    fn metadata(&self, model_json: &str, names_json: &str) {
        if let Ok(mut env) = self.jvm.attach_current_thread_as_daemon() {
            let a: JObject = match env.new_string(model_json) {
                Ok(s) => s.into(),
                Err(_) => return,
            };
            let b: JObject = match env.new_string(names_json) {
                Ok(s) => s.into(),
                Err(_) => return,
            };
            let _ = env.call_method(
                &self.callback,
                "onModelMetadata",
                "(Ljava/lang/String;Ljava/lang/String;)V",
                &[JValue::Object(&a), JValue::Object(&b)],
            );
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
        }
    }

    fn complete(&self, dir: &str) {
        if let Ok(mut env) = self.jvm.attach_current_thread_as_daemon() {
            let s: JObject = match env.new_string(dir) {
                Ok(x) => x.into(),
                Err(_) => return,
            };
            let _ = env.call_method(
                &self.callback,
                "onFetchComplete",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&s)],
            );
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
        }
    }
}

fn throw(env: &mut JNIEnv<'_>, msg: String) {
    let _ = env.throw_new("java/lang/RuntimeException", msg);
}

fn null_bytes<'a>() -> JByteArray<'a> {
    JObject::null().into()
}

fn null_string<'a>() -> JString<'a> {
    JObject::null().into()
}

fn jbytes_to_vec<'local>(env: &mut JNIEnv, arr: &JByteArray) -> Result<Vec<u8>, String> {
    let len = env
        .get_array_length(arr)
        .map_err(|e| format!("{e:?}"))? as usize;
    let mut buf = vec![0i8; len];
    env.get_byte_array_region(arr, 0, &mut buf)
        .map_err(|e| format!("{e:?}"))?;
    Ok(buf.into_iter().map(|b| b as u8).collect())
}

fn vec_to_jbytes<'a>(env: &mut JNIEnv<'a>, v: &[u8]) -> Result<JByteArray<'a>, String> {
    let arr = env
        .new_byte_array(v.len() as i32)
        .map_err(|e| format!("{e:?}"))?;
    let conv: Vec<i8> = v.iter().map(|b| *b as i8).collect();
    env.set_byte_array_region(&arr, 0, &conv)
        .map_err(|e| format!("{e:?}"))?;
    Ok(arr)
}

fn with_engine<'a, F, R>(env: &mut JNIEnv<'a>, f: F) -> Option<R>
where
    F: FnOnce(&mut JNIEnv<'a>, Arc<Engine>) -> Result<R, String>,
{
    let eng = ENGINE.lock().clone();
    match eng {
        Some(e) => match f(env, e) {
            Ok(r) => Some(r),
            Err(msg) => {
                throw(env, msg);
                None
            }
        },
        None => {
            throw(env, "engine not initialized, call initialize() first".to_string());
            None
        }
    }
}

async fn add_bytes(eng: &Engine, data: Vec<u8>) -> Result<iroh_blobs::Hash, String> {
    if data.len() > MAX_FILE_BYTES {
        return Err("blob too large".to_string());
    }
    let s = stream::once(async move { Ok::<_, std::io::Error>(bytes::Bytes::from(data)) });
    let info = eng
        .store
        .blobs()
        .add_stream(s)
        .await
        .with_tag()
        .await
        .map_err(|e| e.to_string())?;
    Ok(info.hash)
}

async fn download_blob(
    eng: &Engine,
    ticket: &iroh_blobs::ticket::BlobTicket,
) -> Result<iroh_blobs::Hash, String> {
    use iroh_blobs::api::downloader::DownloadProgressItem as Item;
    let hash = ticket.hash();
    let peer = ticket.addr().id;
    let downloader = eng.store.downloader(&eng.endpoint);
    let req = downloader.download(hash, Some(peer));
    let mut stream = tokio::time::timeout(Duration::from_secs(FETCH_TIMEOUT_SECS), req.stream())
        .await
        .map_err(|_| "download timed out".to_string())?
        .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(FETCH_TIMEOUT_SECS), async {
        loop {
            match stream.next().await {
                Some(Item::PartComplete { .. }) => break,
                Some(Item::DownloadError) | Some(Item::ProviderFailed { .. }) => {
                    return Err::<(), String>("download failed".to_string());
                }
                Some(Item::Error(e)) => return Err(e.to_string()),
                Some(_) => continue,
                None => break,
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| "download timed out".to_string())??;
    eng.peers.write().insert(hex_encode(peer.as_bytes()));
    Ok(hash)
}

async fn read_blob(eng: &Engine, hash: iroh_blobs::Hash) -> Result<Vec<u8>, String> {
    let bytes = tokio::time::timeout(
        Duration::from_secs(FETCH_TIMEOUT_SECS),
        eng.store.blobs().get_bytes(hash),
    )
    .await
    .map_err(|_| "read timed out".to_string())?
    .map_err(|_| format!("blob {} missing", hex_encode(hash.as_bytes())))?;
    if bytes.len() > MAX_FILE_BYTES + 1024 * 1024 {
        return Err("blob too large".to_string());
    }
    Ok(bytes.to_vec())
}

fn ticket_for(eng: &Engine, hash: iroh_blobs::Hash) -> String {
    iroh_blobs::ticket::BlobTicket::new(eng.endpoint.addr(), hash, iroh_blobs::BlobFormat::Raw)
        .to_string()
}

fn parse_hash(raw: &[u8]) -> Result<iroh_blobs::Hash, String> {
    if raw.len() != 32 {
        return Err("hash must be 32 bytes".to_string());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(raw);
    Ok(iroh_blobs::Hash::from_bytes(arr))
}

fn str_list(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|f| f.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn new_string_array<'local>(env: &mut JNIEnv<'local>, items: &[String]) -> JObjectArray<'local> {
    let empty = env
        .new_object_array(0, "java/lang/String", JObject::null())
        .unwrap();
    let arr = env
        .new_object_array(items.len() as i32, "java/lang/String", JObject::null())
        .unwrap_or(empty);
    for (i, s) in items.iter().enumerate() {
        if let Ok(js) = env.new_string(s) {
            let obj: JObject = js.into();
            let _ = env.set_object_array_element(&arr, i as i32, obj);
        }
    }
    arr
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_initialize<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    storage_dir: JString<'local>,
    secret_key: JByteArray<'local>,
) -> jboolean {
    let out = catch_unwind(AssertUnwindSafe(|| {
        let dir: String = env.get_string(&storage_dir).map(|s| s.into()).map_err(|e| format!("{e:?}"))?;
        let sk_raw = jbytes_to_vec(&mut env, &secret_key)?;
        let sk = if sk_raw.is_empty() {
            iroh::SecretKey::generate()
        } else {
            let arr: [u8; 32] = sk_raw
                .try_into()
                .map_err(|_| "secret_key must be 32 bytes".to_string())?;
            iroh::SecretKey::from_bytes(&arr)
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir failed: {e}"))?;
        let (endpoint, store) = get_runtime().block_on(async {
            let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
                .secret_key(sk)
                .bind()
                .await
                .map_err(|e| format!("bind failed: {e}"))?;
            let store = iroh_blobs::store::fs::FsStore::load(&dir)
                .await
                .map_err(|e| format!("store failed: {e}"))?;
            Ok::<_, String>((endpoint, store))
        })?;
        *ENGINE.lock() = Some(Arc::new(Engine {
            endpoint,
            store,
            index: RwLock::new(SearchIndex::default()),
            peers: RwLock::new(HashSet::new()),
        }));
        Ok::<_, String>(())
    }));
    match out {
        Ok(Ok(())) => JNI_TRUE,
        Ok(Err(msg)) => {
            throw(&mut env, format!("initialize failed: {msg}"));
            JNI_FALSE
        }
        Err(_) => {
            throw(&mut env, "initialize panicked".to_string());
            JNI_FALSE
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_shutdown<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) {
    let eng = ENGINE.lock().take();
    if let Some(e) = eng {
        get_runtime().block_on(e.endpoint.close());
    }
    *FETCH_CANCEL.lock() = None;
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_endpointId<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JByteArray<'local> {
    match with_engine(&mut env, |_: &mut JNIEnv<'local>, e: Arc<Engine>| {
        Ok::<_, String>(e.endpoint.id().as_bytes().to_vec())
    }) {
        Some(v) => vec_to_jbytes(&mut env, &v).unwrap_or_else(|_| null_bytes()),
        None => null_bytes(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_secretKey<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JByteArray<'local> {
    match with_engine(&mut env, |_: &mut JNIEnv<'local>, e: Arc<Engine>| {
        Ok::<_, String>(e.endpoint.secret_key().to_bytes().to_vec())
    }) {
        Some(v) => vec_to_jbytes(&mut env, &v).unwrap_or_else(|_| null_bytes()),
        None => null_bytes(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_blobAdd<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    data: JByteArray<'local>,
) -> JByteArray<'local> {
    let res: Option<Vec<u8>> = with_engine(&mut env, |env: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let bytes = jbytes_to_vec(env, &data)?;
        let h = get_runtime().block_on(add_bytes(&e, bytes))?;
        Ok(h.as_bytes().to_vec())
    });
    match res {
        Some(v) => vec_to_jbytes(&mut env, &v).unwrap_or_else(|_| null_bytes()),
        None => null_bytes(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_blobGet<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    hash: JByteArray<'local>,
) -> JByteArray<'local> {
    let res: Option<Vec<u8>> = with_engine(&mut env, |env: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let raw = jbytes_to_vec(env, &hash)?;
        let h = parse_hash(&raw)?;
        get_runtime().block_on(read_blob(&e, h))
    });
    match res {
        Some(v) => vec_to_jbytes(&mut env, &v).unwrap_or_else(|_| null_bytes()),
        None => null_bytes(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_blobFetch<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    ticket_str: JString<'local>,
) -> JByteArray<'local> {
    let res: Option<Vec<u8>> = with_engine(&mut env, |env: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let ticket_raw: String = env
            .get_string(&ticket_str)
            .map_err(|e| format!("{e:?}"))?
            .into();
        let ticket: iroh_blobs::ticket::BlobTicket = ticket_raw
            .trim()
            .parse()
            .map_err(|_| "Invalid connection token provided.".to_string())?;
        get_runtime()
            .block_on(async move {
                let hash = download_blob(&e, &ticket).await?;
                read_blob(&e, hash).await
            })
    });
    match res {
        Some(v) => vec_to_jbytes(&mut env, &v).unwrap_or_else(|_| null_bytes()),
        None => null_bytes(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_blobHas<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    hash: JByteArray<'local>,
) -> jboolean {
    let res: Option<bool> = with_engine(&mut env, |env: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let raw = jbytes_to_vec(env, &hash)?;
        let h = parse_hash(&raw)?;
        get_runtime()
            .block_on(async { e.store.blobs().has(h).await })
            .map_err(|e| e.to_string())
    });
    match res {
        Some(true) => JNI_TRUE,
        _ => JNI_FALSE,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_ticketFor<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    hash: JByteArray<'local>,
) -> JString<'local> {
    let res: Option<String> = with_engine(&mut env, |env: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let raw = jbytes_to_vec(env, &hash)?;
        let h = parse_hash(&raw)?;
        let has: bool = get_runtime()
            .block_on(async { e.store.blobs().has(h).await })
            .map_err(|e| e.to_string())?;
        if !has {
            return Err("blob not stored locally".to_string());
        }
        Ok(ticket_for(&e, h))
    });
    match res {
        Some(s) => env.new_string(s).unwrap_or_else(|_| null_string()),
        None => null_string(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_ticketInfo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    ticket_str: JString<'local>,
) -> JString<'local> {
    let raw: String = match env.get_string(&ticket_str) {
        Ok(s) => s.into(),
        Err(_) => return null_string(),
    };
    let t: iroh_blobs::ticket::BlobTicket = match raw.trim().parse() {
        Ok(t) => t,
        Err(_) => {
            throw(&mut env, "bad ticket".to_string());
            return null_string();
        }
    };
    let info = format!(
        "{}:{}",
        hex_encode(t.hash().as_bytes()),
        hex_encode(t.addr().id.as_bytes())
    );
    env.new_string(info).unwrap_or_else(|_| null_string())
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_modelPublish<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    metadata_json: JString<'local>,
    files: JObjectArray<'local>,
) -> JString<'local> {
    let res: Option<String> = with_engine(&mut env, |env: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let json: String = env
            .get_string(&metadata_json)
            .map(|s| s.into())
            .map_err(|e| format!("{e:?}"))?;
        let n = env
            .get_array_length(&files)
            .map_err(|e| format!("{e:?}"))? as usize;
        if n == 0 || n > MAX_FILES {
            return Err("bad file count".to_string());
        }
        let mut v: serde_json::Value =
            serde_json::from_str(&json).map_err(|_| "bad model json".to_string())?;
        let names = v
            .get("files")
            .and_then(|f| f.as_array())
            .ok_or_else(|| "metadata missing files".to_string())?;
        if names.len() != n {
            return Err("file count mismatch".to_string());
        }
        let mut datas = Vec::with_capacity(n);
        for i in 0..n {
            let item: JObject = env
                .get_object_array_element(&files, i as i32)
                .map_err(|e| format!("{e:?}"))?;
            let item_arr: JByteArray = item.into();
            datas.push(jbytes_to_vec(env, &item_arr)?);
        }
        let mut hashes = Vec::with_capacity(n);
        let mut tickets = Vec::with_capacity(n);
        let mut sizes = Vec::with_capacity(n);
        for data in datas {
            sizes.push(data.len() as i64);
            let h = get_runtime().block_on(add_bytes(&e, data))?;
            tickets.push(ticket_for(&e, h));
            hashes.push(hex_encode(h.as_bytes()));
        }
        let obj = v
            .as_object_mut()
            .ok_or_else(|| "metadata must be object".to_string())?;
        obj.insert(
            "hashes".into(),
            serde_json::Value::Array(hashes.into_iter().map(serde_json::Value::String).collect()),
        );
        obj.insert(
            "tickets".into(),
            serde_json::Value::Array(
                tickets.into_iter().map(serde_json::Value::String).collect(),
            ),
        );
        obj.insert(
            "sizes".into(),
            serde_json::Value::Array(sizes.into_iter().map(|s| serde_json::json!(s)).collect()),
        );
        let final_json = serde_json::to_string(&v).map_err(|e| e.to_string())?;
        let mh = get_runtime().block_on(add_bytes(&e, final_json.clone().into_bytes()))?;
        index_add(&e.index, mh, &final_json);
        Ok(ticket_for(&e, mh))
    });
    match res {
        Some(s) => env.new_string(s).unwrap_or_else(|_| null_string()),
        None => null_string(),
    }
}

fn publish_files_inner(
    eng: &Engine,
    metadata_json: &str,
    paths: &[String],
    reporter: Option<&Reporter>,
) -> Result<String, String> {
    let report = |pct: i32, msg: String| {
        if let Some(r) = reporter {
            r.progress(6, pct, 0, 0, &msg, false);
        }
    };
    let mut v: serde_json::Value =
        serde_json::from_str(metadata_json).map_err(|_| "bad model json".to_string())?;
    let names = v
        .get("files")
        .and_then(|f| f.as_array())
        .ok_or_else(|| "metadata missing files".to_string())?;
    if names.len() != paths.len() || paths.is_empty() || paths.len() > MAX_FILES {
        return Err("file count mismatch".to_string());
    }
    let mut hashes = Vec::with_capacity(paths.len());
    let mut tickets = Vec::with_capacity(paths.len());
    let mut sizes = Vec::with_capacity(paths.len());
    for (i, path) in paths.iter().enumerate() {
        let size = std::fs::metadata(path)
            .map_err(|e| format!("stat failed: {e}"))?
            .len() as i64;
        if size < 0 || size as usize > MAX_FILE_BYTES {
            return Err("file too large".to_string());
        }
        sizes.push(size);
        report(
            (i as i32 * 90) / paths.len() as i32,
            format!("Importing file {}/{}...", i + 1, paths.len()),
        );
        let h = get_runtime().block_on(async {
            eng.store
                .blobs()
                .add_path(path)
                .with_tag()
                .await
                .map(|info| info.hash)
                .map_err(|e| e.to_string())
        })?;
        tickets.push(ticket_for(eng, h));
        hashes.push(hex_encode(h.as_bytes()));
    }
    let obj = v
        .as_object_mut()
        .ok_or_else(|| "metadata must be object".to_string())?;
    obj.insert(
        "hashes".into(),
        serde_json::Value::Array(hashes.into_iter().map(serde_json::Value::String).collect()),
    );
    obj.insert(
        "tickets".into(),
        serde_json::Value::Array(
            tickets.into_iter().map(serde_json::Value::String).collect(),
        ),
    );
    obj.insert(
        "sizes".into(),
        serde_json::Value::Array(sizes.into_iter().map(|s| serde_json::json!(s)).collect()),
    );
    let final_json = serde_json::to_string(&v).map_err(|e| e.to_string())?;
    report(95, "Indexing model...".to_string());
    let mh = get_runtime().block_on(add_bytes(eng, final_json.clone().into_bytes()))?;
    index_add(&eng.index, mh, &final_json);
    report(100, "Published.".to_string());
    Ok(ticket_for(eng, mh))
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_modelPublishFiles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    metadata_json: JString<'local>,
    paths: JObjectArray<'local>,
    callback: JObject<'local>,
) -> JString<'local> {
    let res: Option<String> = with_engine(&mut env, |env: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let json: String = env
            .get_string(&metadata_json)
            .map(|s| s.into())
            .map_err(|e| format!("{e:?}"))?;
        let n = env
            .get_array_length(&paths)
            .map_err(|e| format!("{e:?}"))? as usize;
        if n == 0 || n > MAX_FILES {
            return Err("bad file count".to_string());
        }
        let mut list = Vec::with_capacity(n);
        for i in 0..n {
            let item: JObject = env
                .get_object_array_element(&paths, i as i32)
                .map_err(|e| format!("{e:?}"))?;
            let js: JString = item.into();
            let p: String = env
                .get_string(&js)
                .map(|s| s.into())
                .map_err(|e| format!("{e:?}"))?;
            list.push(p);
        }
        let rep = match env.new_global_ref(callback) {
            Ok(r) => match env.get_java_vm() {
                Ok(vm) => Some(Reporter::new(vm, r, 500)),
                Err(_) => None,
            },
            Err(_) => None,
        };
        publish_files_inner(&e, &json, &list, rep.as_ref())
    });
    match res {
        Some(s) => env.new_string(s).unwrap_or_else(|_| null_string()),
        None => null_string(),
    }
}

fn spawn_cancel_flag() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let mut guard = FETCH_CANCEL.lock();
    if let Some(prev) = guard.replace(flag.clone()) {
        prev.store(true, Ordering::Relaxed);
    }
    flag
}

fn clear_cancel_flag(flag: &Arc<AtomicBool>) {
    let mut guard = FETCH_CANCEL.lock();
    if let Some(cur) = guard.as_ref() {
        if Arc::ptr_eq(cur, flag) || cur.load(Ordering::Relaxed) {
            *guard = None;
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_modelFetch<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    ticket_str: JString<'local>,
    dir_str: JString<'local>,
    callback: JObject<'local>,
) {
    let _ = catch_unwind(AssertUnwindSafe(move || {
        let ticket_raw: String = match env.get_string(&ticket_str) {
            Ok(s) => s.into(),
            Err(_) => return,
        };
        let dir: String = match env.get_string(&dir_str) {
            Ok(s) => s.into(),
            Err(_) => return,
        };
        let jvm = match env.get_java_vm() {
            Ok(v) => v,
            Err(_) => return,
        };
        let callback_ref = match env.new_global_ref(callback) {
            Ok(r) => r,
            Err(_) => return,
        };
        let eng = match ENGINE.lock().clone() {
            Some(e) => e,
            None => return,
        };
        let reporter = Arc::new(Reporter::new(jvm, callback_ref, 500));
        let cancel = spawn_cancel_flag();
        get_runtime().spawn(async move {
            run_fetch(eng, reporter, cancel.clone(), ticket_raw, dir).await;
            clear_cancel_flag(&cancel);
        });
    }));
}

async fn run_fetch(
    eng: Arc<Engine>,
    reporter: Arc<Reporter>,
    cancel: Arc<AtomicBool>,
    ticket_raw: String,
    dir: String,
) {
    let cancelled = || cancel.load(Ordering::Relaxed);
    let ticket: iroh_blobs::ticket::BlobTicket = match ticket_raw.trim().parse() {
        Ok(t) => t,
        Err(_) => {
            reporter.progress(-1, 0, 0, 0, "Invalid connection token provided.", true);
            return;
        }
    };
    reporter.progress(1, 0, 0, 0, "Connecting directly to remote peer...", true);
    let meta_hash = match download_blob(&eng, &ticket).await {
        Ok(h) => h,
        Err(e) => {
            reporter.progress(-1, 0, 0, 0, &format!("Metadata fetch failed: {e}"), true);
            return;
        }
    };
    let meta_bytes = match read_blob(&eng, meta_hash).await {
        Ok(b) => b,
        Err(e) => {
            reporter.progress(-1, 0, 0, 0, &format!("Metadata read failed: {e}"), true);
            return;
        }
    };
    let meta_json = match String::from_utf8(meta_bytes) {
        Ok(s) => s,
        Err(_) => {
            reporter.progress(-1, 0, 0, 0, "Metadata not utf-8.", true);
            return;
        }
    };
    index_add(&eng.index, meta_hash, &meta_json);
    let v: serde_json::Value = match serde_json::from_str(&meta_json) {
        Ok(v) => v,
        Err(_) => {
            reporter.progress(-1, 0, 0, 0, "Bad metadata json.", true);
            return;
        }
    };
    let names = str_list(&v, "files");
    let tickets = str_list(&v, "tickets");
    if names.is_empty() || names.len() != tickets.len() {
        reporter.progress(-1, 0, 0, 0, "Metadata missing file tickets.", true);
        return;
    }
    let total: i64 = v
        .get("sizes")
        .and_then(|f| f.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).sum())
        .unwrap_or(-1);
    let names_json = serde_json::to_string(&names).unwrap_or_default();
    reporter.metadata(&meta_json, &names_json);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        reporter.progress(-1, 0, 0, 0, &format!("mkdir failed: {e}"), true);
        return;
    }
    let dir_path = std::path::PathBuf::from(&dir);
    let mut downloaded: i64 = 0;
    for (i, name) in names.iter().enumerate() {
        if cancelled() {
            reporter.progress(-3, 0, 0, 0, "Transfer cancelled by user.", true);
            return;
        }
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            reporter.progress(-1, 0, 0, 0, "Unsafe file name in metadata.", true);
            return;
        }
        let ft: iroh_blobs::ticket::BlobTicket = match tickets[i].trim().parse() {
            Ok(x) => x,
            Err(_) => {
                reporter.progress(-1, 0, 0, 0, "Bad file ticket.", true);
                return;
            }
        };
        let fh = match download_blob(&eng, &ft).await {
            Ok(h) => h,
            Err(e) => {
                reporter.progress(-1, 0, 0, 0, &format!("File fetch failed: {e}"), true);
                return;
            }
        };
        let data = match read_blob(&eng, fh).await {
            Ok(d) => d,
            Err(e) => {
                reporter.progress(-1, 0, 0, 0, &format!("File read failed: {e}"), true);
                return;
            }
        };
        downloaded += data.len() as i64;
        if std::fs::write(dir_path.join(name), &data).is_err() {
            reporter.progress(-1, 0, 0, 0, "Write failed.", true);
            return;
        }
        let pct = if total > 0 {
            ((downloaded as f64 / total as f64) * 100.0).clamp(0.0, 99.0) as i32
        } else {
            50
        };
        reporter.progress(
            4,
            pct,
            downloaded,
            total,
            &format!("Syncing assets securely... ({}%)", pct),
            false,
        );
    }
    reporter.progress(5, 100, downloaded, total, "Assets synced successfully.", true);
    reporter.complete(&dir);
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_cancelFetch<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jboolean {
    let mut guard = FETCH_CANCEL.lock();
    if let Some(flag) = guard.take() {
        flag.store(true, Ordering::Relaxed);
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_searchQuery<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    keyword: JString<'local>,
) -> JObjectArray<'local> {
    let empty: JObjectArray = new_string_array(&mut env, &[]);
    let kw: String = match env.get_string(&keyword) {
        Ok(s) => s.into(),
        Err(_) => return empty,
    };
    let res: Option<Vec<String>> = with_engine(&mut env, |_: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let idx = e.index.read();
        let out: Vec<String> = idx
            .kw
            .get(&kw.trim().to_lowercase())
            .map(|s| s.iter().map(|h| hex_encode(h.as_bytes())).collect())
            .unwrap_or_default();
        Ok(out)
    });
    match res {
        Some(list) => new_string_array(&mut env, &list),
        None => empty,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_searchGetMetadata<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    hash_hex: JString<'local>,
) -> JString<'local> {
    let res: Option<String> = with_engine(&mut env, |env: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let hs: String = env
            .get_string(&hash_hex)
            .map(|s| s.into())
            .map_err(|e| format!("{e:?}"))?;
        let raw = hex_decode(&hs).ok_or_else(|| "bad hash hex".to_string())?;
        let h = parse_hash(&raw)?;
        e.index
            .read()
            .get(&h)
            .ok_or_else(|| "model not indexed".to_string())
    });
    match res {
        Some(s) => env.new_string(s).unwrap_or_else(|_| null_string()),
        None => null_string(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_syncAnnounce<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let res: Option<String> = with_engine(&mut env, |_: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let entries: Vec<iroh_blobs::Hash> =
            e.index.read().meta.keys().take(MAX_ANNOUNCE_MODELS).cloned().collect();
        let mut models = Vec::with_capacity(entries.len());
        for h in &entries {
            models.push(serde_json::json!({
                "h": hex_encode(h.as_bytes()),
                "t": ticket_for(&e, *h),
            }));
        }
        let ann = serde_json::json!({"v": 1, "models": models}).to_string();
        let h = get_runtime().block_on(add_bytes(&e, ann.into_bytes()))?;
        Ok(ticket_for(&e, h))
    });
    match res {
        Some(s) => env.new_string(s).unwrap_or_else(|_| null_string()),
        None => null_string(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_syncMerge<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    ticket_str: JString<'local>,
) -> JString<'local> {
    let res: Option<String> = with_engine(&mut env, |env: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let raw: String = env
            .get_string(&ticket_str)
            .map(|s| s.into())
            .map_err(|e| format!("{e:?}"))?;
        let t: iroh_blobs::ticket::BlobTicket =
            raw.trim().parse().map_err(|_| "bad ticket".to_string())?;
        let ah = get_runtime().block_on(download_blob(&e, &t))?;
        let ab = get_runtime().block_on(read_blob(&e, ah))?;
        let ann: serde_json::Value =
            serde_json::from_slice(&ab).map_err(|_| "bad announcement".to_string())?;
        let models = ann
            .get("models")
            .and_then(|m| m.as_array())
            .ok_or_else(|| "bad announcement".to_string())?;
        let mut tickets = Vec::new();
        for m in models.iter().take(MAX_SYNC_MODELS) {
            let (h, mt) = match (
                m.get("h").and_then(|x| x.as_str()),
                m.get("t").and_then(|x| x.as_str()),
            ) {
                (Some(a), Some(b)) => (a, b),
                _ => continue,
            };
            let raw_h = match hex_decode(h) {
                Some(v) => v,
                None => continue,
            };
            let hash = match parse_hash(&raw_h) {
                Ok(x) => x,
                Err(_) => continue,
            };
            if e.index.read().meta.contains_key(&hash) {
                continue;
            }
            let mkt: iroh_blobs::ticket::BlobTicket = match mt.trim().parse() {
                Ok(x) => x,
                Err(_) => continue,
            };
            let mh = match get_runtime().block_on(download_blob(&e, &mkt)) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let mb = match get_runtime().block_on(read_blob(&e, mh)) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let ms = match String::from_utf8(mb) {
                Ok(x) => x,
                Err(_) => continue,
            };
            if serde_json::from_str::<serde_json::Value>(&ms).is_err() {
                continue;
            }
            index_add(&e.index, hash, &ms);
            tickets.push(mt.to_string());
        }
        Ok(serde_json::json!({
            "new_models": tickets.len(),
            "model_tickets": tickets,
        })
        .to_string())
    });
    match res {
        Some(s) => env.new_string(s).unwrap_or_else(|_| null_string()),
        None => null_string(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_knownPeers<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JObjectArray<'local> {
    let res: Option<Vec<String>> = with_engine(&mut env, |_: &mut JNIEnv<'local>, e: Arc<Engine>| {
        let mut v: Vec<String> = e.peers.read().iter().cloned().collect();
        v.sort();
        Ok(v)
    });
    match res {
        Some(list) => new_string_array(&mut env, &list),
        None => new_string_array(&mut env, &[]),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_cancelDownload<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jboolean {
    let mut guard = FETCH_CANCEL.lock();
    if let Some(flag) = guard.take() {
        flag.store(true, Ordering::Relaxed);
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_initializeAndDownload<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    storage_dir: JString<'local>,
    ticket_str: JString<'local>,
    callback: JObject<'local>,
) {
    let _ = catch_unwind(AssertUnwindSafe(move || {
        let path_str: String = match env.get_string(&storage_dir) {
            Ok(js) => js.into(),
            Err(_) => return,
        };
        let ticket_raw: String = match env.get_string(&ticket_str) {
            Ok(js) => js.into(),
            Err(_) => return,
        };
        let jvm = match env.get_java_vm() {
            Ok(v) => v,
            Err(_) => return,
        };
        let callback_ref = match env.new_global_ref(callback) {
            Ok(r) => r,
            Err(_) => return,
        };
        let eng = match ENGINE.lock().clone() {
            Some(e) => e,
            None => {
                let rep = Reporter::new(jvm, callback_ref, 500);
                rep.progress(-1, 0, 0, 0, "Engine not initialized. Call initialize() first.", true);
                return;
            }
        };
        let reporter = Arc::new(Reporter::new(jvm, callback_ref, 500));
        let cancel = spawn_cancel_flag();
        get_runtime().spawn(async move {
            reporter.progress(1, 0, 0, 0, "Optimizing network routes...", true);
            let ticket: iroh_blobs::ticket::BlobTicket = match ticket_raw.trim().parse() {
                Ok(t) => t,
                Err(_) => {
                    reporter.progress(-1, 0, 0, 0, "Invalid connection token provided.", true);
                    return;
                }
            };
            reporter.progress(2, 5, 0, 0, "Connecting directly to remote peer...", true);
            let hash = match download_blob(&eng, &ticket).await {
                Ok(h) => h,
                Err(e) => {
                    reporter.progress(
                        -1,
                        0,
                        0,
                        0,
                        &format!("Secure pathway negotiation failed: {e}"),
                        true,
                    );
                    return;
                }
            };
            if cancel.load(Ordering::Relaxed) {
                reporter.progress(-3, 0, 0, 0, "Transfer cancelled by user.", true);
                return;
            }
            reporter.progress(3, 10, 0, 0, "Secure peer connection established.", true);
            let data = match read_blob(&eng, hash).await {
                Ok(d) => d,
                Err(e) => {
                    reporter.progress(-1, 0, 0, 0, &format!("Read failed: {e}"), true);
                    return;
                }
            };
            let name = format!("{}.bin", hex_encode(hash.as_bytes()));
            let dest = std::path::PathBuf::from(&path_str).join(&name);
            if std::fs::create_dir_all(&path_str)
                .and_then(|_| std::fs::write(&dest, &data))
                .is_err()
            {
                reporter.progress(-1, 0, 0, 0, "Write failed.", true);
                return;
            }
            reporter.progress(5, 100, 0, 0, "Assets synced successfully.", true);
            reporter.complete(dest.to_string_lossy().as_ref());
            clear_cancel_flag(&cancel);
        });
    }));
}

fn sanitize_filename(name: &str) -> Option<String> {
    let base = std::path::Path::new(name.trim())
        .file_name()?
        .to_string_lossy()
        .into_owned();
    if base.is_empty()
        || base.len() > 128
        || base.starts_with('.')
        || base.contains('\0')
    {
        return None;
    }
    Some(base)
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_downloadToPath<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    storage_dir: JString<'local>,
    ticket_str: JString<'local>,
    file_name: JString<'local>,
    callback: JObject<'local>,
) {
    let _ = catch_unwind(AssertUnwindSafe(move || {
        let path_str: String = match env.get_string(&storage_dir) {
            Ok(js) => js.into(),
            Err(_) => return,
        };
        let ticket_raw: String = match env.get_string(&ticket_str) {
            Ok(js) => js.into(),
            Err(_) => return,
        };
        let name_raw: String = match env.get_string(&file_name) {
            Ok(js) => js.into(),
            Err(_) => return,
        };
        let jvm = match env.get_java_vm() {
            Ok(v) => v,
            Err(_) => return,
        };
        let callback_ref = match env.new_global_ref(callback) {
            Ok(r) => r,
            Err(_) => return,
        };
        let eng = match ENGINE.lock().clone() {
            Some(e) => e,
            None => {
                let rep = Reporter::new(jvm, callback_ref, 500);
                rep.progress(-1, 0, 0, 0, "Engine not initialized. Call initialize() first.", true);
                return;
            }
        };
        let reporter = Arc::new(Reporter::new(jvm, callback_ref, 500));
        let name = match sanitize_filename(&name_raw) {
            Some(n) => n,
            None => {
                reporter.progress(-1, 0, 0, 0, "Unsafe file name.", true);
                return;
            }
        };
        let cancel = spawn_cancel_flag();
        get_runtime().spawn(async move {
            reporter.progress(1, 0, 0, 0, "Optimizing network routes...", true);
            let ticket: iroh_blobs::ticket::BlobTicket = match ticket_raw.trim().parse() {
                Ok(t) => t,
                Err(_) => {
                    reporter.progress(-1, 0, 0, 0, "Invalid connection token provided.", true);
                    return;
                }
            };
            reporter.progress(2, 5, 0, 0, "Connecting directly to remote peer...", true);
            let hash = match download_blob(&eng, &ticket).await {
                Ok(h) => h,
                Err(e) => {
                    reporter.progress(
                        -1,
                        0,
                        0,
                        0,
                        &format!("Secure pathway negotiation failed: {e}"),
                        true,
                    );
                    return;
                }
            };
            if cancel.load(Ordering::Relaxed) {
                reporter.progress(-3, 0, 0, 0, "Transfer cancelled by user.", true);
                return;
            }
            let data = match read_blob(&eng, hash).await {
                Ok(d) => d,
                Err(e) => {
                    reporter.progress(-1, 0, 0, 0, &format!("Read failed: {e}"), true);
                    return;
                }
            };
            let dest = std::path::PathBuf::from(&path_str).join(&name);
            if std::fs::create_dir_all(&path_str)
                .and_then(|_| std::fs::write(&dest, &data))
                .is_err()
            {
                reporter.progress(-1, 0, 0, 0, "Write failed.", true);
                return;
            }
            reporter.progress(5, 100, 0, 0, "Assets synced successfully.", true);
            reporter.complete(dest.to_string_lossy().as_ref());
            clear_cancel_flag(&cancel);
        });
    }));
}
