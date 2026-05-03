pub const OFP_VERSION_1_5: u8 = 0x06;
pub const OFP_HEADER_LEN: usize = 8;

pub const OFPT_HELLO: u8 = 0;
pub const OFPT_ERROR: u8 = 1;
pub const OFPT_FEATURES_REQUEST: u8 = 5;
pub const OFPT_FEATURES_REPLY: u8 = 6;
pub const OFPT_BARRIER_REQUEST: u8 = 20;
pub const OFPT_BARRIER_REPLY: u8 = 21;
pub const OFPT_FLOW_MOD: u8 = 14;

pub const OFPP_IN_PORT: u32 = 0xffff_fff8;
pub const OFPP_ANY: u32 = 0xffff_ffff;
pub const OFPG_ANY: u32 = 0xffff_ffff;

pub const OFP_NO_BUFFER: u32 = 0xffff_ffff;
pub const OFPTT_ALL: u8 = 0xff;

pub const OFPMT_OXM: u16 = 1;
pub const OFPXMC_OPENFLOW_BASIC: u16 = 0x8000;

pub const OFPXMT_OFB_IN_PORT: u8 = 0;
pub const OFPXMT_OFB_ETH_DST: u8 = 3;
pub const OFPXMT_OFB_ETH_SRC: u8 = 4;
pub const OFPXMT_OFB_ETH_TYPE: u8 = 5;
pub const OFPXMT_OFB_IP_PROTO: u8 = 10;
pub const OFPXMT_OFB_IPV4_SRC: u8 = 11;
pub const OFPXMT_OFB_IPV4_DST: u8 = 12;
pub const OFPXMT_OFB_ICMPV4_TYPE: u8 = 19;
pub const OFPXMT_OFB_ARP_OP: u8 = 21;
pub const OFPXMT_OFB_ARP_SPA: u8 = 22;
pub const OFPXMT_OFB_ARP_TPA: u8 = 23;
pub const OFPXMT_OFB_ARP_SHA: u8 = 24;
pub const OFPXMT_OFB_ARP_THA: u8 = 25;

pub const OFPIT_APPLY_ACTIONS: u16 = 4;

pub const OFPAT_OUTPUT: u16 = 0;
pub const OFPAT_SET_FIELD: u16 = 25;
pub const OFPAT_COPY_FIELD: u16 = 28;
pub const OFPAT_DEC_NW_TTL: u16 = 24;

pub const OFPFC_ADD: u8 = 0;
pub const OFPFC_DELETE: u8 = 3;
pub const OFP_FLOW_PERMANENT: u16 = 0;

pub const ETH_TYPE_IPV4: u16 = 0x0800;
pub const ETH_TYPE_ARP: u16 = 0x0806;
