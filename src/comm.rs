use crate::cli::{Mode, Transport};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

/// Declares the benchmark a client wants to run. Sent by the client as the first message on a
/// new connection, before either side has built any RDMA resources for that connection.
#[derive(Serialize, Deserialize, Debug)]
pub struct BenchmarkRequest {
    pub transport: Transport,
    pub mode: Mode,
    pub msg_size: usize,
    pub iterations: usize,
    pub tx_depth: usize,
}

/// The server's reply to a `BenchmarkRequest`.
#[derive(Serialize, Deserialize, Debug)]
pub enum HandshakeAck {
    Ok {
        endpoint: ibverbs::QueuePairEndpoint,
    },
    Unsupported(String),
}

/// The client's queue pair endpoint, sent back to the server after the client has consumed the
/// server's `HandshakeAck::Ok`.
#[derive(Serialize, Deserialize, Debug)]
pub struct ClientEndpoint {
    pub endpoint: ibverbs::QueuePairEndpoint,
}

/// A single out-of-band TCP connection used to exchange handshake messages and barriers before
/// (and after) an RDMA benchmark run.
pub struct Conn {
    write_stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl Conn {
    fn new(stream: TcpStream) -> Result<Self> {
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            write_stream: stream,
            reader,
        })
    }

    pub fn send_json<T: Serialize>(&mut self, msg: &T) -> Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        self.write_stream.write_all(line.as_bytes())?;
        Ok(())
    }

    pub fn recv_json<T: DeserializeOwned>(&mut self) -> Result<T> {
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        if line.is_empty() {
            return Err("peer closed the connection while waiting for a message".into());
        }
        Ok(serde_json::from_str(&line)?)
    }

    /// A one-byte round trip both sides call at the same logical point, so neither proceeds
    /// past it until the other has reached it too.
    pub fn sync(&mut self) -> Result<()> {
        self.write_stream.write_all(&[0u8])?;
        let mut buf = [0u8; 1];
        self.reader.read_exact(&mut buf)?;
        Ok(())
    }
}

pub fn listen(port: u16) -> Result<TcpListener> {
    Ok(TcpListener::bind(("0.0.0.0", port))?)
}

pub fn accept_one(listener: &TcpListener) -> Result<Conn> {
    let (stream, _addr) = listener.accept()?;
    Conn::new(stream)
}

pub fn connect(host: &str, port: u16) -> Result<Conn> {
    let stream = TcpStream::connect((host, port))?;
    Conn::new(stream)
}
