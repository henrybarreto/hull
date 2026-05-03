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
