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
