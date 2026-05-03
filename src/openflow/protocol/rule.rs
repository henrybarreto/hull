use super::constants::{
    OFP_FLOW_PERMANENT, OFP_HEADER_LEN, OFP_NO_BUFFER, OFP_VERSION_1_5, OFPFC_ADD, OFPFC_DELETE,
    OFPP_ANY, OFPT_FLOW_MOD,
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
