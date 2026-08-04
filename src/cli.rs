use clap::{Args, Parser, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(name = "rust-rdma-bench")]
pub enum Cli {
    /// Wait for client connections and serve benchmark runs.
    Server(ServerArgs),
    /// Connect to a server and run a single benchmark.
    Client(ClientArgs),
}

#[derive(Args, Debug)]
pub struct ServerArgs {
    /// TCP port for the out-of-band handshake.
    #[arg(long, default_value_t = 18515)]
    pub port: u16,

    /// RDMA device name (e.g. "rxe0"). Defaults to the first device ibverbs::devices() returns.
    #[arg(long)]
    pub device: Option<String>,

    /// Keep accepting connections and serving benchmark runs one after another instead of
    /// exiting after the first.
    #[arg(long)]
    pub listen: bool,
}

#[derive(Args, Debug)]
pub struct ClientArgs {
    /// Server address to connect to.
    #[arg(long)]
    pub host: String,

    /// TCP port for the out-of-band handshake.
    #[arg(long, default_value_t = 18515)]
    pub port: u16,

    /// RDMA device name (e.g. "rxe0"). Defaults to the first device ibverbs::devices() returns.
    #[arg(long)]
    pub device: Option<String>,

    #[arg(long, value_enum, default_value_t = Transport::Rc)]
    pub transport: Transport,

    #[arg(long, value_enum, default_value_t = Mode::Bandwidth)]
    pub mode: Mode,

    /// Message size in bytes for each send/receive.
    #[arg(long, default_value_t = 65536)]
    pub size: usize,

    /// Number of messages to exchange.
    #[arg(long, default_value_t = 1000)]
    pub iterations: usize,

    /// Number of sends/receives allowed to be outstanding at once.
    #[arg(long, default_value_t = 32)]
    pub tx_depth: usize,
}

#[derive(ValueEnum, Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    Rc,
    Uc,
    Ud,
}

#[derive(ValueEnum, Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Bandwidth,
    Latency,
    Accuracy,
}
