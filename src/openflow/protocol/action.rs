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
