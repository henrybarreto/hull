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
