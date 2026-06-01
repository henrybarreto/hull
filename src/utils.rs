use uuid::Uuid;

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

pub fn stable_uuid(kind: &str, values: &[&str]) -> Uuid {
    let mut key = String::from("hull:");
    key.push_str(kind);
    for value in values {
        key.push(':');
        key.push_str(value);
    }

    Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_uuid_is_reproducible() {
        let a = stable_uuid("switch", &["test-sw"]);
        let b = stable_uuid("switch", &["test-sw"]);
        let c = stable_uuid("router", &["test-sw"]);

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn deterministic_mac_is_stable() {
        let a = generate_deterministic_mac("r0", "sw0");
        let b = generate_deterministic_mac("r0", "sw0");
        assert_eq!(a, b);
        assert!(a.starts_with("02:"));
    }
}
