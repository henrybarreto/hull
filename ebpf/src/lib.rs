#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::{TC_ACT_PIPE, TC_ACT_SHOT},
    macros::{classifier, map},
    maps::{HashMap, LpmTrie, ProgramArray, lpm_trie::Key},
    programs::TcContext,
};
use aya_ebpf_bindings::helpers::bpf_redirect;
use common::{ArpKey, Interface, RouteEntry};

#[map]
static JUMP_TABLE: ProgramArray = ProgramArray::with_max_entries(8, 0);

#[map]
static MAC_TABLE: HashMap<[u8; 6], u32> = HashMap::with_max_entries(4096, 0);

#[map]
static BRIDGE_TABLE: HashMap<u32, u32> = HashMap::with_max_entries(256, 0);

#[map]
static ROUTE_TABLE: LpmTrie<[u8; 4], RouteEntry> = LpmTrie::with_max_entries(1024, 0);

#[map]
static DEFAULT_ROUTES: HashMap<u32, RouteEntry> = HashMap::with_max_entries(256, 0);

#[map]
static INTERFACES: HashMap<u32, Interface> = HashMap::with_max_entries(64, 0);

#[map]
static ARP_TABLE: HashMap<ArpKey, [u8; 6]> = HashMap::with_max_entries(4096, 0);

const TAIL_SWITCH: u32 = 1;
const TAIL_ROUTER: u32 = 2;

const ETH_HDR_LEN: usize = 14;
const IPV4_HDR_LEN: usize = 20;
const IP_TTL_OFF: usize = ETH_HDR_LEN + 8;
const IP_PROTO_OFF: usize = ETH_HDR_LEN + 9;
const IP_CSUM_OFF: usize = ETH_HDR_LEN + 10;
const IP_SRC_OFF: usize = ETH_HDR_LEN + 12;
const IP_DST_OFF: usize = ETH_HDR_LEN + 16;
const IP_FRAG_OFF: usize = ETH_HDR_LEN + 6;
const ICMP_TYPE_OFF: usize = ETH_HDR_LEN + IPV4_HDR_LEN;
const ICMP_CSUM_OFF: usize = ETH_HDR_LEN + IPV4_HDR_LEN + 2;
const ICMP_ID_OFF: usize = ETH_HDR_LEN + IPV4_HDR_LEN + 4;
const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_ARP: u16 = 0x0806;
const IP_PROTO_ICMP: u8 = 1;
const ICMP_ECHO_REQUEST: u8 = 8;
const ICMP_ECHO_REPLY: u8 = 0;
const ARP_REQUEST: u16 = 1;
const ARP_REPLY: u16 = 2;
#[inline(always)]
fn ipv4_csum_replace_addr(ctx: &mut TcContext, old_ip: u32, new_ip: u32) {
    let old_be = old_ip.to_be();
    let new_be = new_ip.to_be();
    let _ = ctx.l3_csum_replace(IP_CSUM_OFF, u64::from(old_be), u64::from(new_be), 4);
}

#[inline(always)]
fn ipv4_csum_replace_ttl(ctx: &mut TcContext, old_ttl: u8, new_ttl: u8, proto: u8) {
    let old_word = u16::from(old_ttl) << 8 | u16::from(proto);
    let new_word = u16::from(new_ttl) << 8 | u16::from(proto);
    let _ = ctx.l3_csum_replace(
        IP_CSUM_OFF,
        u64::from(old_word.to_be()),
        u64::from(new_word.to_be()),
        2,
    );
}

fn get_ifindex(ctx: &TcContext) -> u32 {
    let skb_ptr = ctx.skb.skb;
    unsafe { (*skb_ptr).ifindex }
}

const NEXT_SWITCH: i32 = -1;

fn switch_frame(ctx: &TcContext) -> Result<i32, i64> {
    let ethertype = match ctx.load::<u16>(12) {
        Ok(v) => u16::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    if ethertype != ETHERTYPE_ARP && ethertype != ETHERTYPE_IPV4 {
        return Ok(TC_ACT_PIPE);
    }

    let ifindex = get_ifindex(ctx);
    let src_mac = match ctx.load::<[u8; 6]>(6) {
        Ok(v) => v,
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    let _ = MAC_TABLE.insert(&src_mac, &ifindex, 0);

    let dst_mac = match ctx.load::<[u8; 6]>(0) {
        Ok(v) => v,
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    if let Some(out_ifindex) = unsafe { MAC_TABLE.get(&dst_mac) } {
        if *out_ifindex != ifindex {
            let ret = unsafe { bpf_redirect(*out_ifindex, 0) };
            if ret >= 0 {
                return Ok(ret as i32);
            }
        }
    }

    Ok(TC_ACT_SHOT)
}

fn route_frame(ctx: &mut TcContext) -> Result<i32, i64> {
    let ethertype = match ctx.load::<u16>(12) {
        Ok(v) => u16::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };

    if ethertype == ETHERTYPE_ARP {
        let action = handle_arp(ctx)?;
        if action != TC_ACT_PIPE {
            return Ok(action);
        }
        return Ok(NEXT_SWITCH);
    }

    if ethertype != ETHERTYPE_IPV4 {
        return Ok(NEXT_SWITCH);
    }

    let ifindex = get_ifindex(ctx);
    let interface = match unsafe { INTERFACES.get(&ifindex) } {
        Some(i) => *i,
        None => return Ok(NEXT_SWITCH),
    };

    let ip_dst = match ctx.load::<u32>(IP_DST_OFF) {
        Ok(v) => u32::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    if ip_dst == interface.ip {
        let eth_dst = match ctx.load::<[u8; 6]>(0) {
            Ok(v) => v,
            Err(_) => return Ok(TC_ACT_PIPE),
        };
        if eth_dst != interface.mac {
            return Ok(NEXT_SWITCH);
        }
        let action = handle_icmp(ctx, &interface)?;
        if action != TC_ACT_PIPE {
            return Ok(action);
        }
        return Ok(NEXT_SWITCH);
    }

    forward(ctx)
}

fn handle_arp(ctx: &mut TcContext) -> Result<i32, i64> {
    let ifindex = get_ifindex(ctx);
    let bridge_id = match unsafe { BRIDGE_TABLE.get(&ifindex) } {
        Some(id) => *id,
        None => return Ok(TC_ACT_PIPE),
    };
    let target_ip = match ctx.load::<u32>(ETH_HDR_LEN + 24) {
        Ok(v) => u32::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    let oper = match ctx.load::<u16>(ETH_HDR_LEN + 6) {
        Ok(v) => u16::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    if oper != ARP_REQUEST {
        return Ok(TC_ACT_PIPE);
    }
    let target_mac = if let Some(gw) = unsafe { INTERFACES.get(&ifindex) } {
        if target_ip == gw.ip {
            gw.mac
        } else {
            let key = ArpKey {
                bridge_id,
                ip: target_ip,
            };
            match unsafe { ARP_TABLE.get(&key) } {
                Some(mac) => *mac,
                None => return Ok(TC_ACT_PIPE),
            }
        }
    } else {
        let key = ArpKey {
            bridge_id,
            ip: target_ip,
        };
        match unsafe { ARP_TABLE.get(&key) } {
            Some(mac) => *mac,
            None => return Ok(TC_ACT_PIPE),
        }
    };

    let sender_mac = match ctx.load::<[u8; 6]>(ETH_HDR_LEN + 8) {
        Ok(v) => v,
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    let sender_ip = match ctx.load::<u32>(ETH_HDR_LEN + 14) {
        Ok(v) => u32::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };

    let oper_reply: u16 = ARP_REPLY;
    let _ = ctx.store(ETH_HDR_LEN + 6, &oper_reply.to_be_bytes(), 0);
    let _ = ctx.store(ETH_HDR_LEN + 8, &target_mac, 0);
    let _ = ctx.store(ETH_HDR_LEN + 14, &target_ip.to_be_bytes(), 0);
    let _ = ctx.store(ETH_HDR_LEN + 18, &sender_mac, 0);
    let _ = ctx.store(ETH_HDR_LEN + 24, &sender_ip.to_be_bytes(), 0);
    let _ = ctx.store(0, &sender_mac, 0);
    let _ = ctx.store(6, &target_mac, 0);

    let ret = unsafe { bpf_redirect(ifindex, 0) };
    if ret >= 0 {
        Ok(ret as i32)
    } else {
        Ok(TC_ACT_PIPE)
    }
}

fn handle_icmp(ctx: &mut TcContext, gw: &Interface) -> Result<i32, i64> {
    let icmp_type = match ctx.load::<u8>(ICMP_TYPE_OFF) {
        Ok(v) => v,
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    if icmp_type != ICMP_ECHO_REQUEST {
        return Ok(TC_ACT_PIPE);
    }
    let ifindex = get_ifindex(ctx);
    let eth_src = match ctx.load::<[u8; 6]>(6) {
        Ok(v) => v,
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    let ip_src = match ctx.load::<u32>(IP_SRC_OFF) {
        Ok(v) => u32::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    let ip_dst = match ctx.load::<u32>(IP_DST_OFF) {
        Ok(v) => u32::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    let _ = ctx.store(0, &eth_src, 0);
    let _ = ctx.store(6, &gw.mac, 0);
    let _ = ctx.store(IP_SRC_OFF, &ip_dst.to_be_bytes(), 0);
    ipv4_csum_replace_addr(ctx, ip_src, ip_dst);
    let _ = ctx.store(IP_DST_OFF, &ip_src.to_be_bytes(), 0);
    ipv4_csum_replace_addr(ctx, ip_dst, ip_src);
    let _ = ctx.l4_csum_replace(ICMP_CSUM_OFF, u64::from(ICMP_ECHO_REQUEST), u64::from(ICMP_ECHO_REPLY), 2);
    let icmp_reply: u8 = ICMP_ECHO_REPLY;
    let _ = ctx.store(ICMP_TYPE_OFF, &icmp_reply, 0);
    let ret = unsafe { bpf_redirect(ifindex, 0) };
    if ret >= 0 {
        Ok(ret as i32)
    } else {
        Ok(TC_ACT_PIPE)
    }
}

fn forward(ctx: &mut TcContext) -> Result<i32, i64> {
    let ttl = match ctx.load::<u8>(IP_TTL_OFF) {
        Ok(v) => v,
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    if ttl <= 1 {
        return Ok(TC_ACT_SHOT);
    }
    let frag_off = match ctx.load::<u16>(IP_FRAG_OFF) {
        Ok(v) => u16::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    if frag_off & 0x3FFF != 0 {
        return Ok(TC_ACT_PIPE);
    }
    let dst_ip = match ctx.load::<u32>(IP_DST_OFF) {
        Ok(v) => u32::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    if dst_ip == 0 || dst_ip == 0xFFFF_FFFF {
        return Ok(TC_ACT_PIPE);
    }
    let protocol = match ctx.load::<u8>(IP_PROTO_OFF) {
        Ok(v) => v,
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    let in_ifindex = get_ifindex(ctx);
    let route = match unsafe { DEFAULT_ROUTES.get(&in_ifindex) } {
        Some(r) => *r,
        None => {
            let key = Key::new(32, dst_ip.to_be_bytes());
            let default_key = Key::new(0, [0u8; 4]);
            match ROUTE_TABLE.get(&key) {
                Some(r) => *r,
                None => match ROUTE_TABLE.get(&default_key) {
                    Some(r) => *r,
                    None => return Ok(TC_ACT_PIPE),
                },
            }
        }
    };

    let original_src_ip = match ctx.load::<u32>(IP_SRC_OFF) {
        Ok(v) => u32::from_be(v),
        Err(_) => return Ok(TC_ACT_PIPE),
    };
    if !ipv4_prefix_match(original_src_ip, route.src_network, route.src_prefix_len) {
        return Ok(TC_ACT_PIPE);
    }

    let new_ttl = ttl - 1;
    let _ = ctx.store(IP_TTL_OFF, &new_ttl, 0);
    ipv4_csum_replace_ttl(ctx, ttl, new_ttl, protocol);
    let _ = ctx.store(0, &route.next_hop_mac, 0);
    let _ = ctx.store(6, &route.src_mac, 0);

    let ret = unsafe { bpf_redirect(route.out_ifindex, 0) };
    if ret >= 0 { Ok(ret as i32) } else { Ok(TC_ACT_PIPE) }
}

fn ipv4_prefix_match(ip: u32, network: u32, prefix_len: u32) -> bool {
    if prefix_len == 0 {
        return true;
    }
    if prefix_len > 32 {
        return false;
    }
    let mask = u32::MAX << (32 - prefix_len);
    (ip & mask) == (network & mask)
}

#[classifier]
pub fn hull_ingress(ctx: TcContext) -> i32 {
    let ethertype = match ctx.load::<u16>(12) {
        Ok(v) => u16::from_be(v),
        Err(_) => return TC_ACT_PIPE,
    };
    if ethertype == ETHERTYPE_ARP || ethertype == ETHERTYPE_IPV4 {
        unsafe {
            let _ = JUMP_TABLE.tail_call(&ctx, TAIL_ROUTER);
        }
        unsafe {
            let _ = JUMP_TABLE.tail_call(&ctx, TAIL_SWITCH);
        }
        return TC_ACT_PIPE;
    }
    unsafe {
        let _ = JUMP_TABLE.tail_call(&ctx, TAIL_SWITCH);
    }
    TC_ACT_PIPE
}

#[classifier]
pub fn hull_switch(ctx: TcContext) -> i32 {
    switch_frame(&ctx).unwrap_or(TC_ACT_PIPE)
}

#[classifier]
pub fn hull_router(mut ctx: TcContext) -> i32 {
    match route_frame(&mut ctx) {
        Ok(NEXT_SWITCH) | Ok(TC_ACT_PIPE) => {
            unsafe {
                let _ = JUMP_TABLE.tail_call(&ctx, TAIL_SWITCH);
            }
            switch_frame(&ctx).unwrap_or(TC_ACT_PIPE)
        }
        Ok(action) => action,
        Err(_) => TC_ACT_PIPE,
    }
}

#[cfg(not(any(test, target_arch = "x86_64")))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}
