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
}
