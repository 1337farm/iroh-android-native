use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use futures_util::StreamExt;
use jni::objects::{GlobalRef, JClass, JObject, JString};
use jni::sys::{jboolean, jint, JNI_FALSE, JNI_TRUE};
use jni::{JNIEnv, JavaVM};
use parking_lot::Mutex;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static ACTIVE_CANCELLATION_TOKEN: Mutex<Option<CancellationToken>> = Mutex::new(None);

fn get_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("Failed to initialize multi-threaded Tokio runtime")
    })
}

struct SafeProgressReporter {
    jvm: JavaVM,
    callback: GlobalRef,
    last_reported: Mutex<Instant>,
    throttle_duration: Duration,
}

impl SafeProgressReporter {
    fn new(jvm: JavaVM, callback: GlobalRef, throttle_millis: u64) -> Self {
        Self {
            jvm,
            callback,
            last_reported: Mutex::new(Instant::now() - Duration::from_millis(throttle_millis)),
            throttle_duration: Duration::from_millis(throttle_millis),
        }
    }

    fn report(&self, status_code: i32, progress_pct: i32, message: &str, force: bool) {
        if !force {
            let mut last = self.last_reported.lock();
            if last.elapsed() < self.throttle_duration {
                return;
            }
            *last = Instant::now();
        }

        if let Ok(mut env) = self.jvm.attach_current_thread_as_daemon() {
            if let Ok(j_str) = env.new_string(message) {
                let status_val = jint::from(status_code);
                let progress_val = jint::from(progress_pct);
                let _ = env.call_method(
                    &self.callback,
                    "onTransferProgress",
                    "(IILjava/lang/String;)V",
                    &[status_val.into(), progress_val.into(), (&j_str).into()],
                );
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
            }
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_cancelDownload(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let mut token_lock = ACTIVE_CANCELLATION_TOKEN.lock();
        if let Some(token) = token_lock.take() {
            token.cancel();
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    })).unwrap_or(JNI_FALSE);

    let mut token_lock = ACTIVE_CANCELLATION_TOKEN.lock();
    if let Some(token) = token_lock.take() {
        token.cancel();
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_irohapp_IrohBridge_initializeAndDownload(
    mut env: JNIEnv,
    _class: JClass,
    storage_dir: JString,
    ticket_str: JString,
    callback: JObject,
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

        let reporter = Arc::new(SafeProgressReporter::new(jvm, callback_ref, 500));
        let cancel_token = CancellationToken::new();

        {
            let mut token_guard = ACTIVE_CANCELLATION_TOKEN.lock();
            if let Some(prev_token) = token_guard.replace(cancel_token.clone()) {
                prev_token.cancel();
            }
        }

        let rt = get_runtime();
        let cancel_child = cancel_token.clone();

        rt.spawn(async move {
            reporter.report(1, 0, "Optimizing network routes...", true);

            let endpoint = match iroh::Endpoint::bind(iroh::endpoint::presets::N0).await {
                Ok(e) => e,
                Err(_) => {
                    reporter.report(-1, 0, "Internal network initialization failed.", true);
                    return;
                }
            };

            let store = iroh_blobs::store::mem::MemStore::new();

            let ticket = match ticket_raw.parse::<iroh_blobs::ticket::BlobTicket>() {
                Ok(t) => t,
                Err(_) => {
                    reporter.report(-1, 0, "Invalid connection token provided.", true);
                    return;
                }
            };

            reporter.report(2, 5, "Connecting directly to remote peer...", true);

            let downloader = store.downloader(&endpoint);

            let download_req = downloader.download(ticket.hash(), Some(ticket.addr().id));
            let mut stream = match download_req.stream().await {
                Ok(s) => s,
                Err(_) => {
                    reporter.report(-1, 0, "Secure pathway negotiation failed.", true);
                    return;
                }
            };

            loop {
                tokio::select! {
                    _ = cancel_child.cancelled() => {
                        reporter.report(-3, 0, "Transfer cancelled by user.", true);
                        break;
                    }
                    next_progress = stream.next() => {
                        match next_progress {
                            Some(progress) => match progress {
                                iroh_blobs::api::downloader::DownloadProgressItem::TryProvider { .. } => {
                                    reporter.report(3, 10, "Secure peer connection established.", true);
                                }
                                iroh_blobs::api::downloader::DownloadProgressItem::Progress(offset) => {
                                    // Iroh 1.1 doesn't seem to emit `Found { size }` easily in this stream,
                                    // so we can fallback to an indeterminate progress if size isn't known,
                                    // but we can just report the offset or a clamped 50%
                                    let percent = 50;
                                    reporter.report(4, percent, &format!("Syncing assets securely... ({} bytes)", offset), false);
                                }
                                iroh_blobs::api::downloader::DownloadProgressItem::PartComplete { .. } => {
                                    reporter.report(5, 100, "Assets synced successfully.", true);
                                    break;
                                }
                                iroh_blobs::api::downloader::DownloadProgressItem::DownloadError => {
                                    reporter.report(-2, 0, "Connection dropped unexpectedly", true);
                                    break;
                                }
                                iroh_blobs::api::downloader::DownloadProgressItem::ProviderFailed { .. } => {
                                    reporter.report(-2, 0, "Provider failed", true);
                                    break;
                                }
                                iroh_blobs::api::downloader::DownloadProgressItem::Error(err) => {
                                    reporter.report(-2, 0, &format!("Network stream error: {}", err), true);
                                    break;
                                }
                            },
                            None => {
                                reporter.report(5, 100, "Assets synced successfully.", true);
                                break;
                            }
                        }
                    }
                }
            }

            let mut token_guard = ACTIVE_CANCELLATION_TOKEN.lock();
            if let Some(current) = token_guard.as_ref() {
                if current.is_cancelled() {
                    *token_guard = None;
                }
            }
        });
    }));
}
