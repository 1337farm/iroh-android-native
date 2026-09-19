//! Cross-implementation test vector: NodeId for a fixed seed.
//! flashforge-farm's NodeIdentityTest pins the same vector on the Java side,
//! proving Java (net.i2p eddsa) and Rust (iroh SecretKey) derive the identical
//! NodeId (Ed25519 pubkey, lowercase hex) from the same 32-byte seed.
use iroh::SecretKey;

fn node_id_hex(seed: &[u8; 32]) -> String {
    let sk = SecretKey::from_bytes(seed);
    sk.public().as_bytes().iter().map(|b| format!("{:02x}", b)).collect()
}

#[test]
fn fixed_seed_node_id() {
    let seed = [7u8; 32];
    let got = node_id_hex(&seed);
    // Pinned vector (generated 2026-09-19, iroh 1.1.0):
    assert_eq!(
        got,
        "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"
    );
    println!("seed0707... -> NodeId {}", got);
}
