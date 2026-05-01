pub mod constants {
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
}

pub mod error {
    use std::fmt::{Display, Formatter};

    #[derive(Debug)]
    pub enum OfErr {
        ShortBuffer,
        UnsupportedVersion(u8),
        InvalidLength(u16),
        UnknownMessageType(u8),
        Io(std::io::Error),
    }

    pub type Result<T> = std::result::Result<T, OfErr>;

    impl Display for OfErr {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::ShortBuffer => write!(f, "short buffer"),
                Self::UnsupportedVersion(version) => {
                    write!(f, "unsupported OpenFlow version: {version:#x}")
                }
                Self::InvalidLength(len) => write!(f, "invalid message length: {len}"),
                Self::UnknownMessageType(msg_type) => {
                    write!(f, "unknown message type: {msg_type}")
                }
                Self::Io(err) => write!(f, "io error: {err}"),
            }
        }
    }

    impl std::error::Error for OfErr {}

    impl From<std::io::Error> for OfErr {
        fn from(value: std::io::Error) -> Self {
            Self::Io(value)
        }
    }
}

pub mod header {
    use super::constants::{OFP_HEADER_LEN, OFP_VERSION_1_5};
    use super::error::{OfErr, Result};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Header {
        pub version: u8,
        pub msg_type: u8,
        pub length: u16,
        pub xid: u32,
    }

    impl Header {
        /// Parse an `OpenFlow` header from raw bytes.
        ///
        /// # Errors
        ///
        /// Returns an error if the buffer is too short, the version is not
        /// `OpenFlow` 1.5, or the encoded length is invalid.
        pub fn parse(buf: &[u8]) -> Result<Self> {
            if buf.len() < OFP_HEADER_LEN {
                return Err(OfErr::ShortBuffer);
            }
            let version = *buf.first().ok_or(OfErr::ShortBuffer)?;
            if version != OFP_VERSION_1_5 {
                return Err(OfErr::UnsupportedVersion(version));
            }
            let msg_type = *buf.get(1).ok_or(OfErr::ShortBuffer)?;
            let length = u16::from_be_bytes([
                *buf.get(2).ok_or(OfErr::ShortBuffer)?,
                *buf.get(3).ok_or(OfErr::ShortBuffer)?,
            ]);
            let xid = u32::from_be_bytes([
                *buf.get(4).ok_or(OfErr::ShortBuffer)?,
                *buf.get(5).ok_or(OfErr::ShortBuffer)?,
                *buf.get(6).ok_or(OfErr::ShortBuffer)?,
                *buf.get(7).ok_or(OfErr::ShortBuffer)?,
            ]);
            if length < 8 {
                return Err(OfErr::InvalidLength(length));
            }
            Ok(Self {
                version,
                msg_type,
                length,
                xid,
            })
        }

        pub fn encode(&self, out: &mut Vec<u8>) {
            out.push(self.version);
            out.push(self.msg_type);
            out.extend_from_slice(&self.length.to_be_bytes());
            out.extend_from_slice(&self.xid.to_be_bytes());
        }
    }
}

pub mod features {
    use super::constants::OFPT_FEATURES_REPLY;
    use super::error::{OfErr, Result};
    use super::header::Header;

    #[derive(Debug, Clone, Default)]
    pub struct Reply;

    impl Reply {
        /// Parse a features reply from raw bytes.
        ///
        /// # Errors
        ///
        /// Returns an error if the frame is too short or the message type is
        /// not a features reply.
        pub fn parse(frame: &[u8]) -> Result<Self> {
            let header = Header::parse(frame)?;
            if header.msg_type != OFPT_FEATURES_REPLY {
                return Err(OfErr::UnknownMessageType(header.msg_type));
            }
            if frame.len() < 32 {
                return Err(OfErr::ShortBuffer);
            }

            Ok(Self)
        }
    }
}

pub mod ofmatch {
    use super::constants::OFPMT_OXM;

    #[derive(Debug, Clone)]
    pub struct Match {
        pub oxms: Vec<Vec<u8>>,
    }

    impl Match {
        pub const fn any() -> Self {
            Self { oxms: Vec::new() }
        }

        pub const fn new(oxms: Vec<Vec<u8>>) -> Self {
            Self { oxms }
        }

        pub fn encode(&self, out: &mut Vec<u8>) {
            let start = out.len();
            out.extend_from_slice(&OFPMT_OXM.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes());
            for oxm in &self.oxms {
                out.extend_from_slice(oxm);
            }
            let actual_len = u16::try_from(out.len() - start).unwrap_or(u16::MAX);
            if let Some(dst) = out.get_mut(start + 2..start + 4) {
                dst.copy_from_slice(&actual_len.to_be_bytes());
            }
            while !out.len().is_multiple_of(8) {
                out.push(0);
            }
        }
    }
}

pub mod oxm {
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
}

pub mod action {
    use super::constants::{OFPAT_COPY_FIELD, OFPAT_DEC_NW_TTL, OFPAT_OUTPUT, OFPAT_SET_FIELD};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Action {
        Output {
            port: u32,
            max_len: u16,
        },
        SetField(Vec<u8>),
        CopyField {
            n_bits: u16,
            src_offset: u16,
            dst_offset: u16,
            oxm_ids: Vec<u8>,
        },
        DecNwTtl,
    }

    impl Action {
        pub const fn output(port: u32) -> Self {
            Self::Output { port, max_len: 0 }
        }

        pub fn encode(&self, out: &mut Vec<u8>) {
            match self {
                Self::Output { port, max_len } => {
                    out.extend_from_slice(&OFPAT_OUTPUT.to_be_bytes());
                    out.extend_from_slice(&16u16.to_be_bytes());
                    out.extend_from_slice(&port.to_be_bytes());
                    out.extend_from_slice(&max_len.to_be_bytes());
                    out.extend_from_slice(&[0u8; 6]);
                }
                Self::SetField(field) => {
                    let start = out.len();
                    out.extend_from_slice(&OFPAT_SET_FIELD.to_be_bytes());
                    out.extend_from_slice(&0u16.to_be_bytes());
                    out.extend_from_slice(field);
                    while !out.len().is_multiple_of(8) {
                        out.push(0);
                    }
                    let len = u16::try_from(out.len() - start).unwrap_or(u16::MAX);
                    if let Some(dst) = out.get_mut(start + 2..start + 4) {
                        dst.copy_from_slice(&len.to_be_bytes());
                    }
                }
                Self::CopyField {
                    n_bits,
                    src_offset,
                    dst_offset,
                    oxm_ids,
                } => {
                    let start = out.len();
                    out.extend_from_slice(&OFPAT_COPY_FIELD.to_be_bytes());
                    out.extend_from_slice(&0u16.to_be_bytes());
                    out.extend_from_slice(&n_bits.to_be_bytes());
                    out.extend_from_slice(&src_offset.to_be_bytes());
                    out.extend_from_slice(&dst_offset.to_be_bytes());
                    out.extend_from_slice(&[0u8; 2]);
                    out.extend_from_slice(oxm_ids);
                    while !out.len().is_multiple_of(8) {
                        out.push(0);
                    }
                    let len = u16::try_from(out.len() - start).unwrap_or(u16::MAX);
                    if let Some(dst) = out.get_mut(start + 2..start + 4) {
                        dst.copy_from_slice(&len.to_be_bytes());
                    }
                }
                Self::DecNwTtl => {
                    out.extend_from_slice(&OFPAT_DEC_NW_TTL.to_be_bytes());
                    out.extend_from_slice(&8u16.to_be_bytes());
                    out.extend_from_slice(&[0u8; 4]);
                }
            }
        }
    }
}

pub mod instruction {
    use super::action::Action;
    use super::constants::OFPIT_APPLY_ACTIONS;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Instruction {
        ApplyActions(Vec<Action>),
    }

    impl Instruction {
        pub const fn apply_actions(actions: Vec<Action>) -> Self {
            Self::ApplyActions(actions)
        }

        pub fn encode(&self, out: &mut Vec<u8>) {
            match self {
                Self::ApplyActions(actions) => {
                    let start = out.len();
                    out.extend_from_slice(&OFPIT_APPLY_ACTIONS.to_be_bytes());
                    out.extend_from_slice(&0u16.to_be_bytes());
                    out.extend_from_slice(&[0u8; 4]);
                    for action in actions {
                        action.encode(out);
                    }
                    let len = u16::try_from(out.len() - start).unwrap_or(u16::MAX);
                    if let Some(dst) = out.get_mut(start + 2..start + 4) {
                        dst.copy_from_slice(&len.to_be_bytes());
                    }
                }
            }
        }
    }
}

pub mod rule {
    use super::constants::{
        OFP_FLOW_PERMANENT, OFP_HEADER_LEN, OFP_NO_BUFFER, OFP_VERSION_1_5, OFPFC_ADD,
        OFPFC_DELETE, OFPP_ANY, OFPT_FLOW_MOD,
    };
    use super::header::Header;
    use super::instruction::Instruction;
    use super::ofmatch::Match;

    #[derive(Debug, Clone)]
    pub struct Rule {
        pub xid: u32,
        pub cookie: u64,
        pub cookie_mask: u64,
        pub table_id: u8,
        pub command: u8,
        pub idle_timeout: u16,
        pub hard_timeout: u16,
        pub priority: u16,
        pub buffer_id: u32,
        pub out_port: u32,
        pub out_group: u32,
        pub flags: u16,
        pub importance: u16,
        pub of_match: Match,
        pub instructions: Vec<Instruction>,
    }

    impl Rule {
        pub const fn add(
            xid: u32,
            table_id: u8,
            priority: u16,
            of_match: Match,
            instructions: Vec<Instruction>,
        ) -> Self {
            Self {
                xid,
                cookie: 0,
                cookie_mask: 0,
                table_id,
                command: OFPFC_ADD,
                idle_timeout: OFP_FLOW_PERMANENT,
                hard_timeout: OFP_FLOW_PERMANENT,
                priority,
                buffer_id: OFP_NO_BUFFER,
                out_port: OFPP_ANY,
                out_group: OFPP_ANY,
                flags: 0,
                importance: 0,
                of_match,
                instructions,
            }
        }

        pub const fn delete(xid: u32, table_id: u8, of_match: Match) -> Self {
            Self {
                xid,
                cookie: 0,
                cookie_mask: u64::MAX,
                table_id,
                command: OFPFC_DELETE,
                idle_timeout: OFP_FLOW_PERMANENT,
                hard_timeout: OFP_FLOW_PERMANENT,
                priority: 0,
                buffer_id: OFP_NO_BUFFER,
                out_port: OFPP_ANY,
                out_group: OFPP_ANY,
                flags: 0,
                importance: 0,
                of_match,
                instructions: Vec::new(),
            }
        }

        #[must_use]
        pub const fn with_cookie(mut self, cookie: u64) -> Self {
            self.cookie = cookie;
            self
        }

        #[must_use]
        pub const fn with_cookie_mask(mut self, cookie_mask: u64) -> Self {
            self.cookie_mask = cookie_mask;
            self
        }

        #[must_use]
        pub const fn with_out_port(mut self, out_port: u32) -> Self {
            self.out_port = out_port;
            self
        }

        #[must_use]
        pub const fn with_out_group(mut self, out_group: u32) -> Self {
            self.out_group = out_group;
            self
        }

        pub fn encode(&self) -> Vec<u8> {
            let mut body = Vec::new();
            body.extend_from_slice(&self.cookie.to_be_bytes());
            body.extend_from_slice(&self.cookie_mask.to_be_bytes());
            body.push(self.table_id);
            body.push(self.command);
            body.extend_from_slice(&self.idle_timeout.to_be_bytes());
            body.extend_from_slice(&self.hard_timeout.to_be_bytes());
            body.extend_from_slice(&self.priority.to_be_bytes());
            body.extend_from_slice(&self.buffer_id.to_be_bytes());
            body.extend_from_slice(&self.out_port.to_be_bytes());
            body.extend_from_slice(&self.out_group.to_be_bytes());
            body.extend_from_slice(&self.flags.to_be_bytes());
            body.extend_from_slice(&self.importance.to_be_bytes());
            self.of_match.encode(&mut body);
            for inst in &self.instructions {
                inst.encode(&mut body);
            }
            let len = u16::try_from(OFP_HEADER_LEN + body.len()).unwrap_or(u16::MAX);
            let mut out = Vec::with_capacity(usize::from(len));
            Header {
                version: OFP_VERSION_1_5,
                msg_type: OFPT_FLOW_MOD,
                length: len,
                xid: self.xid,
            }
            .encode(&mut out);
            out.extend_from_slice(&body);
            out
        }
    }
}

pub mod message {
    use super::constants::{OFPT_BARRIER_REPLY, OFPT_ERROR, OFPT_FEATURES_REPLY, OFPT_HELLO};
    use super::error::{OfErr, Result};
    use super::features::Reply;
    use super::header::Header;

    #[derive(Debug)]
    pub enum Message {
        Hello,
        Error {
            error_type: u16,
            code: u16,
            data: Vec<u8>,
        },
        FeaturesReply(Reply),
        BarrierReply {
            xid: u32,
        },
        Ignored,
    }

    /// Decode a raw `OpenFlow` message.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame is truncated or contains an invalid
    /// header or payload for the encoded message type.
    pub fn decode(frame: &[u8]) -> Result<Message> {
        let header = Header::parse(frame)?;
        let msg_len = header.length as usize;
        if frame.len() < msg_len {
            return Err(OfErr::ShortBuffer);
        }
        match header.msg_type {
            OFPT_HELLO => Ok(Message::Hello),
            OFPT_ERROR => {
                if msg_len < 12 {
                    return Err(OfErr::InvalidLength(header.length));
                }
                Ok(Message::Error {
                    error_type: u16::from_be_bytes(
                        frame
                            .get(8..10)
                            .ok_or(OfErr::ShortBuffer)?
                            .try_into()
                            .map_err(|_| OfErr::ShortBuffer)?,
                    ),
                    code: u16::from_be_bytes(
                        frame
                            .get(10..12)
                            .ok_or(OfErr::ShortBuffer)?
                            .try_into()
                            .map_err(|_| OfErr::ShortBuffer)?,
                    ),
                    data: frame.get(12..msg_len).ok_or(OfErr::ShortBuffer)?.to_vec(),
                })
            }
            OFPT_FEATURES_REPLY => Ok(Message::FeaturesReply(Reply::parse(frame)?)),
            OFPT_BARRIER_REPLY => Ok(Message::BarrierReply { xid: header.xid }),
            _ => Ok(Message::Ignored),
        }
    }
}
