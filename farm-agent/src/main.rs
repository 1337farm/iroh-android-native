//! farm-agent: on-printer dial-back for USB reverse-pairing.
//!
//! Usage: farm-agent pair --phone-addr <path> [--timeout <secs>]
//!
//! Reads the phone.addr payload staged by the farm app, dials the phone's node
//! over iroh on ALPN farm/pair/0, presents the pairing token, and exits 0 only
//! when the phone answers {"ok":true}. Retry/backoff lives in the printer shell
//! harness (flashforge_init.sh); this binary is single-shot.
//!
//! Exit codes: 0 paired, 1 failed/rejected, 2 usage error.

mod pair;

use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 60;
const RESP_MAX_BYTES: usize = 65536;

fn usage() -> ! {
    eprintln!("usage: farm-agent pair --phone-addr <path> [--timeout <secs>]");
    std::process::exit(2);
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) != Some("pair") {
        usage();
    }
    let mut addr_path: Option<String> = None;
    let mut timeout = DEFAULT_TIMEOUT_SECS;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--phone-addr" => {
                i += 1;
                addr_path = args.get(i).cloned();
            }
            "--timeout" => {
                i += 1;
                timeout = args
                    .get(i)
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(DEFAULT_TIMEOUT_SECS)
                    .clamp(5, 300);
            }
            _ => usage(),
        }
        i += 1;
    }
    let path = match addr_path {
        Some(p) => p,
        None => usage(),
    };
    match pair_file(&path, timeout) {
        Ok(true) => {
            println!("paired");
            0
        }
        Ok(false) => {
            eprintln!("pairing rejected by phone");
            1
        }
        Err(e) => {
            eprintln!("pair failed: {e}");
            1
        }
    }
}

fn pair_file(path: &str, timeout_secs: u64) -> Result<bool, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("read phone.addr: {e}"))?;
    let addr = pair::parse_phone_addr(&json)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    rt.block_on(pair_async(&addr, timeout_secs))
}

async fn pair_async(addr: &pair::PhoneAddr, timeout_secs: u64) -> Result<bool, String> {
    let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .bind()
        .await
        .map_err(|e| format!("endpoint bind: {e}"))?;
    let id: iroh::EndpointId = addr
        .node_id
        .parse()
        .map_err(|_| "phone nodeId unparsable".to_string())?;
    let mut target = iroh::EndpointAddr::new(id);
    let relay = addr.relays.first().map(|s| s.trim()).unwrap_or("");
    if !relay.is_empty() {
        let url: iroh::RelayUrl = relay
            .parse()
            .map_err(|_| "phone relay URL unparsable".to_string())?;
        target = target.with_relay_url(url);
    } else if let Some(url) = iroh::defaults::prod::default_relay_map()
        .urls::<Vec<_>>()
        .into_iter()
        .next()
    {
        target = target.with_relay_url(url);
    }
    let conn = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        endpoint.connect(target, addr.alpn.as_bytes()),
    )
    .await
    .map_err(|_| "dial timed out".to_string())?
    .map_err(|e| format!("dial failed: {e}"))?;
    let (mut send, mut recv) = tokio::time::timeout(
        Duration::from_secs(30),
        conn.open_bi(),
    )
    .await
    .map_err(|_| "open stream timed out".to_string())?
    .map_err(|e| format!("open_bi failed: {e}"))?;
    let beacon = pair::build_beacon(&addr.token, &addr.printer_id);
    tokio::time::timeout(Duration::from_secs(30), send.write_all(beacon.as_bytes()))
        .await
        .map_err(|_| "beacon write timed out".to_string())?
        .map_err(|e| format!("beacon write failed: {e}"))?;
    send
        .finish()
        .map_err(|e| format!("beacon finish failed: {e}"))?;
    let bytes = tokio::time::timeout(
        Duration::from_secs(30),
        recv.read_to_end(RESP_MAX_BYTES),
    )
    .await
    .map_err(|_| "response read timed out".to_string())?
    .map_err(|e| format!("response read failed: {e}"))?;
    pair::parse_response(&String::from_utf8_lossy(&bytes))
}
