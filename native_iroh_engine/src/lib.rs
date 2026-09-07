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

            let node = match iroh::node::Node::memory().spawn().await {
                Ok(n) => n,
                Err(_) => {
                    reporter.report(-1, 0, "Internal network initialization failed.", true);
                    return;
                }
            };

            let ticket = match ticket_raw.parse::<iroh::ticket::BlobTicket>() {
                Ok(t) => t,
                Err(_) => {
                    reporter.report(-1, 0, "Invalid connection token provided.", true);
                    return;
                }
            };

            reporter.report(2, 5, "Connecting directly to remote peer...", true);

            let client = node.client();
            let blobs = client.blobs();

            let mut stream = match blobs.download(ticket.hash(), ticket.node_addr().clone()).await {
                Ok(s) => s,
                Err(_) => {
                    reporter.report(-1, 0, "Secure pathway negotiation failed.", true);
                    return;
                }
            };

            let mut discovered_total_size: Option<u64> = None;

            loop {
                tokio::select! {
                    _ = cancel_child.cancelled() => {
                        reporter.report(-3, 0, "Transfer cancelled by user.", true);
                        break;
                    }
                    next_progress = stream.next() => {
                        match next_progress {
                            Some(Ok(progress)) => match progress {
                                iroh::rpc_client::blobs::DownloadProgress::Connected => {
                                    reporter.report(3, 10, "Secure peer connection established.", true);
                                }
                                iroh::rpc_client::blobs::DownloadProgress::Found { size, .. } => {
                                    discovered_total_size = Some(size);
                                }
                                iroh::rpc_client::blobs::DownloadProgress::Progress { offset, .. } => {
                                    let percent = if let Some(total) = discovered_total_size {
                                        if total > 0 {
                                            ((offset as f64 / total as f64) * 100.0).clamp(10.0, 99.0) as i32
                                        } else {
                                            50
                                        }
                                    } else {
                                        50
                                    };
                                    reporter.report(4, percent, &format!("Syncing assets securely... ({}%)", percent), false);
                                }
                                iroh::rpc_client::blobs::DownloadProgress::Done => {
                                    reporter.report(5, 100, "Assets synced successfully.", true);
                                    break;
                                }
                                iroh::rpc_client::blobs::DownloadProgress::Abort(err) => {
                                    reporter.report(-2, 0, &format!("Connection dropped unexpectedly: {}", err), true);
                                    break;
                                }
                                _ => {}
                            },
                            Some(Err(err)) => {
                                reporter.report(-2, 0, &format!("Network stream error: {}", err), true);
                                break;
                            }
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
