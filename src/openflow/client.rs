use std::path::{Path, PathBuf};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;

use super::protocol::constants::{OFPG_ANY, OFPP_ANY, OFPTT_ALL};
use super::protocol::error::OfErr;
use super::protocol::header::Header;
use super::protocol::message::{Message, decode};
use super::protocol::ofmatch::Match;
use super::protocol::rule::Rule;

#[derive(Debug)]
pub enum Error {
    Protocol(OfErr),
    Io(std::io::Error),
    UnexpectedMessage {
        expected: &'static str,
        got: String,
    },
    Remote {
        error_type: u16,
        code: u16,
        data: Vec<u8>,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(err) => write!(f, "OpenFlow protocol error: {err}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::UnexpectedMessage { expected, got } => {
                write!(
                    f,
                    "unexpected OpenFlow message while waiting for {expected}: {got}"
                )
            }
            Self::Remote {
                error_type,
                code,
                data,
            } => write!(
                f,
                "remote OpenFlow error: type={error_type} code={code} data={data:02x?}"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<OfErr> for Error {
    fn from(value: OfErr) -> Self {
        Self::Protocol(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Connection<S> {
    stream: S,
    next_xid: u32,
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub const fn new(stream: S) -> Self {
        Self {
            stream,
            next_xid: 1,
        }
    }

    const fn next_xid(&mut self) -> u32 {
        let xid = self.next_xid;
        self.next_xid = self.next_xid.wrapping_add(1);
        xid
    }

    /// Perform the `OpenFlow` handshake.
    ///
    /// # Errors
    ///
    /// Returns an error if the peer closes the connection, sends an
    /// unexpected message, or replies with an `OpenFlow` error.
    async fn read_frame(&mut self) -> Result<Vec<u8>> {
        let mut header_buf = [0u8; 8];
        self.stream.read_exact(&mut header_buf).await?;
        let header = Header::parse(&header_buf)?;
        let mut frame = Vec::with_capacity(usize::from(header.length));
        frame.extend_from_slice(&header_buf);
        let body_len = usize::from(header.length).saturating_sub(8);
        let mut body = vec![0u8; body_len];
        if body_len > 0 {
            self.stream.read_exact(&mut body).await?;
            frame.extend_from_slice(&body);
        }
        Ok(frame)
    }

    /// Send a barrier request and wait for its reply.
    ///
    /// # Errors
    ///
    /// Returns an error if the barrier request cannot be written or the
    /// expected reply is not received.
    async fn write_frame(&mut self, frame: &[u8]) -> Result<()> {
        self.stream.write_all(frame).await?;
        Ok(())
    }

    async fn read_message(&mut self) -> Result<Message> {
        let frame = self.read_frame().await?;
        Ok(decode(&frame)?)
    }

    async fn wait_for_message<T, F>(&mut self, mut map_message: F) -> Result<T>
    where
        F: FnMut(Message) -> Option<T>,
    {
        loop {
            let msg = self.read_message().await?;
            if let Message::Error {
                error_type,
                code,
                data,
            } = &msg
            {
                return Err(Error::Remote {
                    error_type: *error_type,
                    code: *code,
                    data: data.clone(),
                });
            }
            if let Some(value) = map_message(msg) {
                return Ok(value);
            }
        }
    }

    async fn wait_for_barrier(&mut self, xid: u32) -> Result<()> {
        self.wait_for_message(|msg| match msg {
            Message::BarrierReply { xid: reply_xid } if reply_xid == xid => Some(()),
            _ => None,
        })
        .await
    }

    /// Perform the `OpenFlow` handshake.
    ///
    /// # Errors
    ///
    /// Returns an error if the peer closes the connection, sends an
    /// unexpected message, or replies with an `OpenFlow` error.
    pub async fn handshake(&mut self) -> Result<()> {
        let hello_xid = self.next_xid();
        self.write_frame(&encode_hello(hello_xid)).await?;
        match self.read_message().await? {
            Message::Hello => {}
            Message::Error {
                error_type,
                code,
                data,
            } => {
                return Err(Error::Remote {
                    error_type,
                    code,
                    data,
                });
            }
            got => {
                return Err(Error::UnexpectedMessage {
                    expected: "hello",
                    got: format!("{got:?}"),
                });
            }
        }
        let features_xid = self.next_xid();
        self.write_frame(&encode_features_request(features_xid))
            .await?;
        let _ = self
            .wait_for_message(|msg| match msg {
                Message::FeaturesReply(reply) => Some(reply),
                _ => None,
            })
            .await?;
        Ok(())
    }

    /// Send a barrier request and wait for its reply.
    ///
    /// # Errors
    ///
    /// Returns an error if the barrier request cannot be written or the
    /// expected reply is not received.
    pub async fn send_barrier(&mut self) -> Result<()> {
        let xid = self.next_xid();
        self.write_frame(&encode_barrier_request(xid)).await?;
        self.wait_for_barrier(xid).await
    }

    /// Send a flow-mod message.
    ///
    /// # Errors
    ///
    /// Returns an error if the frame cannot be written to the stream.
    pub async fn send_flow_mod(&mut self, mut flow_mod: Rule) -> Result<()> {
        flow_mod.xid = self.next_xid();
        self.write_frame(&flow_mod.encode()).await
    }

    /// Send a flow-mod message and wait for the following barrier reply.
    ///
    /// # Errors
    ///
    /// Returns an error if either frame cannot be written or the barrier
    /// reply is not received.
    pub async fn add_flow(&mut self, flow_mod: Rule) -> Result<()> {
        self.send_flow_mod(flow_mod).await?;
        self.send_barrier().await
    }

    /// Delete flows matching an optional cookie and wait for the barrier.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete flow-mod or barrier reply fails.
    pub async fn delete_flows(&mut self, cookie: Option<u64>) -> Result<()> {
        let mut flow_mod = Rule::delete(self.next_xid(), OFPTT_ALL, Match::any())
            .with_out_port(OFPP_ANY)
            .with_out_group(OFPG_ANY);
        flow_mod = match cookie {
            Some(cookie) => flow_mod.with_cookie(cookie).with_cookie_mask(u64::MAX),
            None => flow_mod.with_cookie(0).with_cookie_mask(0),
        };
        self.write_frame(&flow_mod.encode()).await?;
        self.send_barrier().await
    }
}

impl Connection<UnixStream> {
    /// Connect to `OpenFlow` on a Unix socket and perform the handshake.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket connection or handshake fails.
    pub async fn connect_unix(path: impl AsRef<Path>) -> Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let mut client = Self::new(stream);
        client.handshake().await?;
        Ok(client)
    }

    /// Connect to `OpenFlow` on an OVS bridge socket and perform the handshake.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket connection or handshake fails.
    pub async fn connect_bridge(bridge: &str) -> Result<Self> {
        Self::connect_unix(bridge_socket_path(bridge)).await
    }
}

fn bridge_socket_path(bridge: &str) -> PathBuf {
    PathBuf::from(format!("/run/openvswitch/{bridge}.mgmt"))
}

fn encode_header(msg_type: u8, xid: u32, length: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(usize::from(length));
    Header {
        version: super::protocol::constants::OFP_VERSION_1_5,
        msg_type,
        length,
        xid,
    }
    .encode(&mut out);
    out
}

fn encode_hello(xid: u32) -> Vec<u8> {
    encode_header(super::protocol::constants::OFPT_HELLO, xid, 8)
}

fn encode_features_request(xid: u32) -> Vec<u8> {
    encode_header(super::protocol::constants::OFPT_FEATURES_REQUEST, xid, 8)
}

fn encode_barrier_request(xid: u32) -> Vec<u8> {
    encode_header(super::protocol::constants::OFPT_BARRIER_REQUEST, xid, 8)
}
