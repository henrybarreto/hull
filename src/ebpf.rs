use anyhow::{Result, anyhow};
use aya::Ebpf;
use aya::maps::{
    MapData,
    hash_map::HashMap as AyaHashMap,
    lpm_trie::{Key as LpmKey, LpmTrie as AyaLpmTrie},
};
use aya::programs::{
    SchedClassifier, TcAttachType,
    tc::{NlOptions, TcAttachOptions},
};
use common::{ArpKey, Interface, RouteEntry};
use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard};
use tracing::{debug, trace};

const INGRESS_PROGRAM: &str = "hull_ingress";

fn is_eexist_error(message: &str) -> bool {
    message.contains("File exists")
        || message.contains("os error 17")
        || message.contains("already attached")
}

fn is_already_loaded_error(message: &str) -> bool {
    message.contains("already loaded")
}

fn load_classifier(ebpf: &mut Ebpf, name: &str) -> Result<()> {
    let program: &mut SchedClassifier = ebpf
        .program_mut(name)
        .ok_or_else(|| anyhow!("{name} program not found"))?
        .try_into()?;
    if let Err(e) = program.load()
        && !is_already_loaded_error(&e.to_string())
    {
        return Err(anyhow!("failed to load program '{name}': {e}"));
    }
    Ok(())
}

fn retry_attach_ingress(program: &mut SchedClassifier, iface: &str) -> Result<()> {
    debug!(interface = %iface, "ingress already attached, replacing");
    let _ = aya::programs::tc::qdisc_detach_program(iface, TcAttachType::Ingress, INGRESS_PROGRAM);

    let retry_options = TcAttachOptions::Netlink(NlOptions::default());
    if let Err(e) = program.attach_with_options(iface, TcAttachType::Ingress, retry_options) {
        if is_eexist_error(&e.to_string()) {
            debug!(interface = %iface, "ingress program already attached, skipping");
            return Ok(());
        }
        return Err(e.into());
    }

    Ok(())
}

pub struct BridgePlane {
    ebpf: Mutex<Ebpf>,
    attached_ifaces: Mutex<HashSet<String>>,
}

impl BridgePlane {
    fn lock_ebpf(&self) -> Result<MutexGuard<'_, Ebpf>> {
        self.ebpf
            .lock()
            .map_err(|e| anyhow!("eBPF state lock poisoned: {e}"))
    }

    fn lock_attached_ifaces(&self) -> Result<MutexGuard<'_, HashSet<String>>> {
        self.attached_ifaces
            .lock()
            .map_err(|e| anyhow!("attached interface lock poisoned: {e}"))
    }

    pub fn load(data: &[u8]) -> Result<Self> {
        let mut ebpf = Ebpf::load(data)?;
        load_classifier(&mut ebpf, INGRESS_PROGRAM)?;

        Ok(Self {
            ebpf: Mutex::new(ebpf),
            attached_ifaces: Mutex::new(HashSet::new()),
        })
    }

    pub fn attach_tap(&self, iface: &str) -> Result<()> {
        debug!(interface = %iface, "attaching eBPF TC programs");

        {
            let attached = self.lock_attached_ifaces()?;
            if attached.contains(iface) {
                trace!(interface = %iface, "eBPF TC program already attached by this daemon");
                return Ok(());
            }
        }

        let mut ebpf = self.lock_ebpf()?;

        match aya::programs::tc::qdisc_add_clsact(iface) {
            Ok(()) => {}
            Err(e) if is_eexist_error(&e.to_string()) => {
                debug!(interface = %iface, "clsact qdisc already exists, skipping");
            }
            Err(e) => return Err(e.into()),
        }

        let ingress: &mut SchedClassifier = ebpf
            .program_mut(INGRESS_PROGRAM)
            .ok_or_else(|| anyhow!("{INGRESS_PROGRAM} program not found"))?
            .try_into()?;
        if let Err(e) = ingress.load()
            && !is_already_loaded_error(&e.to_string())
        {
            return Err(anyhow!("failed to load program '{INGRESS_PROGRAM}': {e}"));
        }

        let _ =
            aya::programs::tc::qdisc_detach_program(iface, TcAttachType::Ingress, INGRESS_PROGRAM);
        let options = TcAttachOptions::Netlink(NlOptions::default());
        match ingress.attach_with_options(iface, TcAttachType::Ingress, options) {
            Ok(_) => {}
            Err(e) if is_eexist_error(&e.to_string()) => {
                retry_attach_ingress(ingress, iface)?;
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        self.lock_attached_ifaces()?.insert(iface.to_string());
        Ok(())
    }

    pub fn detach_tap(&self, iface: &str) -> Result<()> {
        debug!(interface = %iface, "detaching eBPF TC programs");
        let _ =
            aya::programs::tc::qdisc_detach_program(iface, TcAttachType::Ingress, INGRESS_PROGRAM);
        self.lock_attached_ifaces()?.remove(iface);
        Ok(())
    }

    pub fn set_bridge_member(&self, ifindex: u32, bridge_id: u32) -> Result<()> {
        debug!(ifindex, bridge_id, "setting bridge member");
        let mut ebpf = self.lock_ebpf()?;

        let bridge_table = ebpf
            .map_mut("BRIDGE_TABLE")
            .ok_or_else(|| anyhow!("BRIDGE_TABLE map not found"))?;
        let mut bt: AyaHashMap<&mut MapData, u32, u32> = AyaHashMap::try_from(bridge_table)?;
        bt.insert(ifindex, bridge_id, 0)?;
        Ok(())
    }

    pub fn remove_bridge_member(&self, ifindex: u32) -> Result<()> {
        debug!(ifindex, "removing bridge member");
        let mut ebpf = self.lock_ebpf()?;

        if let Some(bridge_table) = ebpf.map_mut("BRIDGE_TABLE") {
            let mut bt: AyaHashMap<&mut MapData, u32, u32> = AyaHashMap::try_from(bridge_table)?;
            let _ = bt.remove(&ifindex);
        }

        for entry in self.dump_mac_table()? {
            if entry.out_ifindex == ifindex {
                let _ = self.remove_mac_entry(entry.mac);
            }
        }

        Ok(())
    }

    pub fn add_route(
        &self,
        src_ip: [u8; 4],
        src_prefix_len: u32,
        dst_ip: [u8; 4],
        prefix_len: u32,
        out_ifindex: u32,
        next_hop_mac: [u8; 6],
        src_mac: [u8; 6],
        flags: u8,
    ) -> Result<()> {
        trace!(?dst_ip, prefix_len, out_ifindex, "adding route");
        let mut ebpf = self.lock_ebpf()?;

        let route_table = ebpf
            .map_mut("ROUTE_TABLE")
            .ok_or_else(|| anyhow!("ROUTE_TABLE map not found"))?;
        let mut rt: AyaLpmTrie<&mut MapData, [u8; 4], RouteEntry> =
            AyaLpmTrie::try_from(route_table)?;

        let key = LpmKey::new(prefix_len, dst_ip);
        let value = RouteEntry {
            out_ifindex,
            src_prefix_len,
            src_network: u32::from_be_bytes(src_ip),
            next_hop_mac,
            src_mac,
            flags,
            _pad: [0u8; 3],
        };

        rt.insert(&key, value, 0)?;
        Ok(())
    }

    pub fn clear_routes(&self) -> Result<()> {
        debug!("clearing eBPF route table");
        let mut ebpf = self.lock_ebpf()?;
        let route_table = ebpf
            .map_mut("ROUTE_TABLE")
            .ok_or_else(|| anyhow!("ROUTE_TABLE map not found"))?;
        let mut rt: AyaLpmTrie<&mut MapData, [u8; 4], RouteEntry> =
            AyaLpmTrie::try_from(route_table)?;

        let keys: Vec<_> = rt
            .iter()
            .filter_map(|item| item.ok().map(|(key, _)| key))
            .collect();
        for key in keys {
            let _ = rt.remove(&key);
        }

        Ok(())
    }

    pub fn clear_arp_entries(&self) -> Result<()> {
        let mut ebpf = self.lock_ebpf()?;
        let arp_table = ebpf
            .map_mut("ARP_TABLE")
            .ok_or_else(|| anyhow!("ARP_TABLE map not found"))?;
        let mut at: AyaHashMap<&mut MapData, ArpKey, [u8; 6]> = AyaHashMap::try_from(arp_table)?;
        let keys: Vec<_> = at
            .iter()
            .filter_map(|item| item.ok().map(|(key, _)| key))
            .collect();
        for key in keys {
            let _ = at.remove(&key);
        }
        Ok(())
    }

    pub fn register_arp_entry(&self, bridge_id: u32, ip: u32, mac: [u8; 6]) -> Result<()> {
        let mut ebpf = self.lock_ebpf()?;
        let arp_table = ebpf
            .map_mut("ARP_TABLE")
            .ok_or_else(|| anyhow!("ARP_TABLE map not found"))?;
        let mut at: AyaHashMap<&mut MapData, ArpKey, [u8; 6]> = AyaHashMap::try_from(arp_table)?;
        at.insert(ArpKey { bridge_id, ip }, mac, 0)?;
        Ok(())
    }

    pub fn register_gateway(&self, ifindex: u32, ip: u32, mac: [u8; 6]) -> Result<()> {
        debug!(ifindex, ?ip, ?mac, "registering gateway");
        let mut ebpf = self.lock_ebpf()?;

        let interfaces = ebpf
            .map_mut("INTERFACES")
            .ok_or_else(|| anyhow!("INTERFACES map not found"))?;
        let mut gi: AyaHashMap<&mut MapData, u32, Interface> = AyaHashMap::try_from(interfaces)?;

        let value = Interface { ip, mac };

        gi.insert(ifindex, value, 0)?;
        Ok(())
    }

    pub fn unregister_gateway(&self, ifindex: u32) -> Result<()> {
        debug!(ifindex, "unregistering gateway");
        let mut ebpf = self.lock_ebpf()?;

        if let Some(interfaces) = ebpf.map_mut("INTERFACES") {
            let mut gi: AyaHashMap<&mut MapData, u32, Interface> =
                AyaHashMap::try_from(interfaces)?;
            let _ = gi.remove(&ifindex);
        }

        if let Some(route_table) = ebpf.map_mut("ROUTE_TABLE") {
            let mut rt: AyaLpmTrie<&mut MapData, [u8; 4], RouteEntry> =
                AyaLpmTrie::try_from(route_table)?;
            let mut to_remove = Vec::new();
            for (key, value) in rt.iter().flatten() {
                if value.out_ifindex == ifindex {
                    to_remove.push(key);
                }
            }
            for key in to_remove {
                let _ = rt.remove(&key);
            }
        }

        Ok(())
    }

    fn dump_mac_table(&self) -> Result<Vec<MacEntry>> {
        let ebpf = self.lock_ebpf()?;
        let mac_table = ebpf
            .map("MAC_TABLE")
            .ok_or_else(|| anyhow!("MAC_TABLE map not found"))?;
        let mac_table: AyaHashMap<&MapData, [u8; 6], u32> = AyaHashMap::try_from(mac_table)?;

        let mut entries = Vec::new();
        for item in mac_table.iter() {
            let (mac, out_ifindex) = item?;
            entries.push(MacEntry { mac, out_ifindex });
        }
        Ok(entries)
    }

    fn remove_mac_entry(&self, mac: [u8; 6]) -> Result<()> {
        let mut ebpf = self.lock_ebpf()?;
        let mac_table = ebpf
            .map_mut("MAC_TABLE")
            .ok_or_else(|| anyhow!("MAC_TABLE map not found"))?;
        let mut mt: AyaHashMap<&mut MapData, [u8; 6], u32> = AyaHashMap::try_from(mac_table)?;
        let _ = mt.remove(&mac);
        Ok(())
    }
}

#[derive(Debug)]
struct MacEntry {
    mac: [u8; 6],
    out_ifindex: u32,
}
