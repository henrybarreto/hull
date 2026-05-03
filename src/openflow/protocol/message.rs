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
