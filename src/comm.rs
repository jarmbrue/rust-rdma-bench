use crate::cli::{Mode, Transport};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;
use std::time::Instant;

/// Whether barrier tracing is on, read once from `RDMA_BENCH_DEBUG`.
///
/// An unmatched `sync()` deadlocks the run instead of failing, and the two halves of a benchmark
/// live in different processes on different machines, so the only way to see which barrier a stuck
/// run is sitting on is to have both sides announce them. Off by default: the tracing writes to
/// stderr from inside the measured section, so it must not be on for a real measurement.
fn trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("RDMA_BENCH_DEBUG").is_some_and(|v| v != "0"))
}

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
    ///
    /// `label` names the barrier for tracing only; it is not sent over the wire, so the two sides
    /// need not agree on it — but keeping their labels recognisably paired is what makes a
    /// `RDMA_BENCH_DEBUG` log of a hung run readable.
    pub fn sync(&mut self, label: &str) -> Result<()> {
        if !trace_enabled() {
            self.write_stream.write_all(&[0u8])?;
            let mut buf = [0u8; 1];
            self.reader.read_exact(&mut buf)?;
            return Ok(());
        }

        eprintln!("[sync] reached  {label}");
        let t0 = Instant::now();
        self.write_stream.write_all(&[0u8])?;
        let mut buf = [0u8; 1];
        // The wait for the peer's byte is where a mispaired barrier hangs, so the time spent here
        // is the interesting number: a barrier that took far longer than the peer's own work is a
        // barrier the peer never reached.
        match self.reader.read_exact(&mut buf) {
            Ok(()) => {
                eprintln!("[sync] passed   {label} (waited {:?})", t0.elapsed());
                Ok(())
            }
            Err(e) => {
                eprintln!("[sync] failed   {label} after {:?}: {e}", t0.elapsed());
                Err(e.into())
            }
        }
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
