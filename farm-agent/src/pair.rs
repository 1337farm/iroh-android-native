//! Pure pairing logic: phone.addr parsing, beacon building, response parsing.
//! No I/O here so everything is unit-testable on any host.

pub const PHONE_ADDR_FORMAT: &str = "flashforge-farm-phone-addr";
pub const PHONE_ADDR_VERSION: u32 = 1;
pub const AGENT_ID: &str = "farm-agent/0.1";

pub struct PhoneAddr {
    pub node_id: String,
    pub relays: Vec<String>,
    pub alpn: String,
    pub token: String,
    pub printer_id: String,
}

pub fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn parse_phone_addr(json: &str) -> Result<PhoneAddr, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "phone.addr is not JSON".to_string())?;
    if v.get("format").and_then(|f| f.as_str()) != Some(PHONE_ADDR_FORMAT) {
        return Err("phone.addr has wrong format".to_string());
    }
    if v.get("version").and_then(|x| x.as_u64()) != Some(PHONE_ADDR_VERSION as u64) {
        return Err("phone.addr has unsupported version".to_string());
    }
    let node_id = v
        .get("nodeId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "phone.addr has no nodeId".to_string())?
        .to_lowercase();
    if !is_hex(&node_id, 64) {
        return Err("phone.addr nodeId must be 64 hex chars".to_string());
    }
    let relays: Vec<String> = v
        .get("relays")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let alpn = v
        .get("alpn")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "phone.addr has no alpn".to_string())?
        .to_string();
    if alpn.is_empty() || alpn.len() > 64 {
        return Err("phone.addr alpn must be 1..64 chars".to_string());
    }
    let token = v
        .get("token")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "phone.addr has no token".to_string())?
        .to_lowercase();
    if !is_hex(&token, 64) {
        return Err("phone.addr token must be 64 hex chars".to_string());
    }
    let printer_id = v
        .get("printerId")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Ok(PhoneAddr {
        node_id,
        relays,
        alpn,
        token,
        printer_id,
    })
}

/// Beacon the printer sends on the pair stream (one JSON line).
pub fn build_beacon(token: &str, printer_id: &str) -> String {
    serde_json::json!({
        "token": token,
        "printer": printer_id,
        "agent": AGENT_ID,
    })
    .to_string()
}

/// Parse the phone's one-line response. Ok(true) = bound.
pub fn parse_response(json: &str) -> Result<bool, String> {
    let v: serde_json::Value =
        serde_json::from_str(json.trim()).map_err(|_| "pair response is not JSON".to_string())?;
    v.get("ok")
        .and_then(|x| x.as_bool())
        .ok_or_else(|| "pair response has no ok flag".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_addr() -> String {
        serde_json::json!({
            "format": PHONE_ADDR_FORMAT,
            "version": 1,
            "nodeId": "ab".repeat(32),
            "relays": [],
            "alpn": "farm/pair/0",
            "token": "cd".repeat(32),
            "printerId": "12345",
            "issuedAt": 1726000000000u64,
        })
        .to_string()
    }

    #[test]
    fn parses_valid_phone_addr() {
        let a = parse_phone_addr(&valid_addr()).expect("valid addr must parse");
        assert_eq!(a.node_id, "ab".repeat(32));
        assert!(a.relays.is_empty());
        assert_eq!(a.alpn, "farm/pair/0");
        assert_eq!(a.token, "cd".repeat(32));
        assert_eq!(a.printer_id, "12345");
    }

    #[test]
    fn rejects_wrong_format_and_version() {
        let bad = valid_addr().replace(PHONE_ADDR_FORMAT, "other");
        assert!(parse_phone_addr(&bad).is_err());
        let bad = valid_addr().replace("\"version\":1", "\"version\":2");
        assert!(parse_phone_addr(&bad).is_err());
    }

    #[test]
    fn rejects_malformed_ids() {
        let bad = valid_addr().replace(&"ab".repeat(32), "xyz");
        assert!(parse_phone_addr(&bad).is_err());
        let bad = valid_addr().replace(&"cd".repeat(32), "12");
        assert!(parse_phone_addr(&bad).is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_phone_addr("not json").is_err());
        assert!(parse_phone_addr("{}").is_err());
    }

    #[test]
    fn beacon_roundtrip() {
        let b = build_beacon(&"cd".repeat(32), "12345");
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["token"], "cd".repeat(32));
        assert_eq!(v["printer"], "12345");
        assert_eq!(v["agent"], AGENT_ID);
    }

    #[test]
    fn response_parsing() {
        assert_eq!(parse_response("{\"ok\":true}\n"), Ok(true));
        assert_eq!(parse_response("{\"ok\":false}"), Ok(false));
        assert!(parse_response("{}").is_err());
        assert!(parse_response("nope").is_err());
    }
}
