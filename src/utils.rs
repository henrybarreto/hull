/// Generate a random locally-administered MAC address.
pub fn generate_random_mac() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    format!(
        "52:54:00:{:02x}:{:02x}:{:02x}",
        rng.r#gen::<u8>(),
        rng.r#gen::<u8>(),
        rng.r#gen::<u8>()
    )
}

/// Generate a deterministic MAC for the router gateway on a given switch.
/// Uses a simple hash of router_name + switch_name to produce a stable,
/// locally-administered MAC address (02:xx:xx:xx:xx:xx).
pub fn generate_deterministic_mac(router_name: &str, switch_name: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    router_name.hash(&mut hasher);
    switch_name.hash(&mut hasher);
    let hash = hasher.finish().to_be_bytes();

    format!(
        "02:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        hash[0], hash[1], hash[2], hash[3], hash[4]
    )
}
