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
