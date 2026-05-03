use super::constants::{
    OFPXMC_OPENFLOW_BASIC, OFPXMT_OFB_ARP_OP, OFPXMT_OFB_ARP_SHA, OFPXMT_OFB_ARP_SPA,
    OFPXMT_OFB_ARP_TPA, OFPXMT_OFB_ETH_DST, OFPXMT_OFB_ETH_SRC, OFPXMT_OFB_ETH_TYPE,
    OFPXMT_OFB_ICMPV4_TYPE, OFPXMT_OFB_IN_PORT, OFPXMT_OFB_IP_PROTO, OFPXMT_OFB_IPV4_DST,
    OFPXMT_OFB_IPV4_SRC,
};

const fn oxm_header(class: u16, field: u8, has_mask: bool, len: u8) -> u32 {
    ((class as u32) << 16) | ((field as u32) << 9) | ((has_mask as u32) << 8) | (len as u32)
}

fn push_oxm(out: &mut Vec<u8>, field: u8, has_mask: bool, value: &[u8]) {
    let len = u8::try_from(value.len()).unwrap_or(u8::MAX);
    let h = oxm_header(OFPXMC_OPENFLOW_BASIC, field, has_mask, len);
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(value);
}

fn push_masked_oxm(out: &mut Vec<u8>, field: u8, value: &[u8], mask: &[u8]) {
    let mut buf = Vec::with_capacity(value.len() * 2);
    buf.extend_from_slice(value);
    buf.extend_from_slice(mask);
    let len = u8::try_from(value.len().saturating_mul(2)).unwrap_or(u8::MAX);
    let h = oxm_header(OFPXMC_OPENFLOW_BASIC, field, true, len);
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&buf);
}

fn push_oxm_id(out: &mut Vec<u8>, field: u8, len: u8) {
    let h = oxm_header(OFPXMC_OPENFLOW_BASIC, field, false, len);
    out.extend_from_slice(&h.to_be_bytes());
}

pub fn copy_field_ids(src_field: u8, src_len: u8, dst_field: u8, dst_len: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    push_oxm_id(&mut out, src_field, src_len);
    push_oxm_id(&mut out, dst_field, dst_len);
    out
}

pub fn in_port(port: u32) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_IN_PORT, false, &port.to_be_bytes());
    out
}

pub fn eth_dst(mac: [u8; 6]) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_ETH_DST, false, &mac);
    out
}

pub fn eth_src(mac: [u8; 6]) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_ETH_SRC, false, &mac);
    out
}

pub fn eth_type(eth_type: u16) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(
        &mut out,
        OFPXMT_OFB_ETH_TYPE,
        false,
        &eth_type.to_be_bytes(),
    );
    out
}

pub fn ipv4_src(ip: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_IPV4_SRC, false, &ip);
    out
}

pub fn ipv4_src_masked(ip: [u8; 4], mask: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    push_masked_oxm(&mut out, OFPXMT_OFB_IPV4_SRC, &ip, &mask);
    out
}

pub fn ipv4_dst(ip: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_IPV4_DST, false, &ip);
    out
}

pub fn ipv4_dst_masked(ip: [u8; 4], mask: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    push_masked_oxm(&mut out, OFPXMT_OFB_IPV4_DST, &ip, &mask);
    out
}

pub fn ip_proto(proto: u8) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_IP_PROTO, false, &[proto]);
    out
}

pub fn icmpv4_type(icmp_type: u8) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_ICMPV4_TYPE, false, &[icmp_type]);
    out
}

pub fn arp_op(op: u16) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_ARP_OP, false, &op.to_be_bytes());
    out
}

pub fn arp_spa(ip: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_ARP_SPA, false, &ip);
    out
}

pub fn arp_tpa(ip: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_ARP_TPA, false, &ip);
    out
}

pub fn arp_sha(mac: [u8; 6]) -> Vec<u8> {
    let mut out = Vec::new();
    push_oxm(&mut out, OFPXMT_OFB_ARP_SHA, false, &mac);
    out
}
