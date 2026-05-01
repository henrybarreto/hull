use anyhow::{Result, anyhow};

/// Flow ownership kinds encoded into the high bits of an OVS cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowCookieKind {
    /// Switch-owned flow cookie.
    Switch = 0x01,
    /// Router-owned flow cookie.
    Router = 0x02,
}

const FLOW_COOKIE_KIND_SHIFT: u64 = 56;
const FLOW_COOKIE_ID_MASK: u64 = 0x00ff_ffff_ffff_ffff;

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
/// Uses a simple hash of `router_name` + `switch_name` to produce a stable,
/// locally-administered MAC address (`02:xx:xx:xx:xx:xx`).
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

/// Derive a stable cookie from a UUID string and a flow ownership kind.
///
/// # Errors
/// Returns an error if `uuid` is not a valid UUID string.
pub fn flow_cookie(kind: FlowCookieKind, uuid: &str) -> Result<u64> {
    let uuid = uuid::Uuid::parse_str(uuid).map_err(|e| anyhow!("invalid uuid '{uuid}': {e}"))?;
    let bytes = uuid.as_bytes();

    let mut id_bytes = [0u8; 8];
    id_bytes[1..].copy_from_slice(&bytes[..7]);
    let object_id = u64::from_be_bytes(id_bytes) & FLOW_COOKIE_ID_MASK;

    Ok(((kind as u64) << FLOW_COOKIE_KIND_SHIFT) | object_id)
}

/// Format a cookie as an exact-match OVS del-flows argument.
pub fn flow_cookie_match(cookie: u64) -> String {
    format!("cookie=0x{cookie:016x}/0xffffffffffffffff")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_cookie_is_stable_for_same_uuid() -> Result<()> {
        let cookie_a = flow_cookie(
            FlowCookieKind::Switch,
            "550e8400-e29b-41d4-a716-446655440000",
        )?;
        let cookie_b = flow_cookie(
            FlowCookieKind::Switch,
            "550e8400-e29b-41d4-a716-446655440000",
        )?;

        assert_eq!(cookie_a, cookie_b);
        Ok(())
    }

    #[test]
    fn flow_cookie_differs_by_kind() -> Result<()> {
        let switch_cookie = flow_cookie(
            FlowCookieKind::Switch,
            "550e8400-e29b-41d4-a716-446655440000",
        )?;
        let router_cookie = flow_cookie(
            FlowCookieKind::Router,
            "550e8400-e29b-41d4-a716-446655440000",
        )?;

        assert_ne!(switch_cookie, router_cookie);
        Ok(())
    }
}
