#![no_std]

#[cfg(not(target_arch = "bpf"))]
use aya::Pod;

#[repr(C)]
pub struct IpPrefix {
    pub prefix_len: u32,
    pub data: [u8; 4],
}

impl IpPrefix {
    pub fn new(ip: u32, prefix_len: u8) -> Self {
        Self {
            prefix_len: prefix_len as u32,
            data: ip.to_be_bytes(),
        }
    }
}

impl Copy for IpPrefix {}
impl Clone for IpPrefix {
    fn clone(&self) -> Self {
        *self
    }
}

#[repr(C)]
pub struct RouteEntry {
    pub out_ifindex: u32,
    pub src_prefix_len: u32,
    pub src_network: u32,
    pub src_mac: [u8; 6],
    pub next_hop_mac: [u8; 6],
    pub flags: u8,
    pub _pad: [u8; 3],
}

impl Copy for RouteEntry {}
impl Clone for RouteEntry {
    fn clone(&self) -> Self {
        *self
    }
}

#[repr(C)]
pub struct Interface {
    pub ip: u32,
    pub mac: [u8; 6],
}

impl Copy for Interface {}
impl Clone for Interface {
    fn clone(&self) -> Self {
        *self
    }
}

#[repr(C)]
pub struct ArpKey {
    pub bridge_id: u32,
    pub ip: u32,
}

impl Copy for ArpKey {}
impl Clone for ArpKey {
    fn clone(&self) -> Self {
        *self
    }
}

#[cfg(not(target_arch = "bpf"))]
unsafe impl Pod for RouteEntry {}

#[cfg(not(target_arch = "bpf"))]
unsafe impl Pod for Interface {}

#[cfg(not(target_arch = "bpf"))]
unsafe impl Pod for ArpKey {}

#[cfg(not(target_arch = "bpf"))]
unsafe impl Pod for IpPrefix {}
