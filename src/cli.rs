use clap::{Args, Parser, ValueEnum};
use serde::{Deserialize, Serialize};

/// Bounds of the default message size sweep. The lower one is the smallest size accuracy mode can
/// identify (it needs room for its 8-byte sequence-number header); the upper one is kept at 64 KiB
/// because accuracy mode registers a `tx_depth`-slot buffer, so its memory region grows with the
/// message size — sweeping higher is fine, but pair it with a smaller `--tx-depth`.
const DEFAULT_MIN_SIZE: usize = 8;
const DEFAULT_MAX_SIZE: usize = 1 << 16;

#[derive(Parser, Debug)]
#[command(name = "rust-rdma-bench")]
pub enum Cli {
    /// Wait for client connections and serve benchmark runs.
    Server(ServerArgs),
    /// Connect to a server and run one benchmark, or a whole suite of them.
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

/// `--mode` and `--size` both take lists, and left out entirely they mean "everything": all three
/// modes, and every power of two from `--min-size` to `--max-size`. So a client given neither runs
/// the complete suite. Since every (mode, size) pair opens its own connection, anything but a
/// single run needs the peer started as `server --listen`.
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

    /// Modes to run, comma-separated. Left out, all of them run.
    #[arg(long, value_enum, value_delimiter = ',')]
    pub mode: Vec<Mode>,

    /// Message sizes in bytes, comma-separated. Left out, the sweep between --min-size and
    /// --max-size is used instead.
    #[arg(long, value_delimiter = ',')]
    pub size: Vec<usize>,

    /// Lower bound of the default power-of-two size sweep.
    #[arg(long, default_value_t = DEFAULT_MIN_SIZE)]
    pub min_size: usize,

    /// Upper bound of the default power-of-two size sweep.
    #[arg(long, default_value_t = DEFAULT_MAX_SIZE)]
    pub max_size: usize,

    /// Number of messages to exchange per run.
    #[arg(long, default_value_t = 1000)]
    pub iterations: usize,

    /// Number of sends/receives allowed to be outstanding at once.
    #[arg(long, default_value_t = 32)]
    pub tx_depth: usize,
}

/// The benchmark matrix a client run expands to: every mode in `modes` once per entry in `sizes`.
/// A single benchmark is just the one-by-one case of that.
pub struct Plan {
    /// Modes to run, in the order given.
    pub modes: Vec<Mode>,
    /// Message sizes in bytes to run each mode at, ascending.
    pub sizes: Vec<usize>,
}

impl Plan {
    /// Whether this is a single explicit benchmark rather than a sweep — the two are reported
    /// differently.
    pub fn is_single_run(&self) -> bool {
        self.modes.len() == 1 && self.sizes.len() == 1
    }
}

impl ClientArgs {
    /// Resolves the CLI's optional lists into the matrix to actually run.
    pub fn plan(&self) -> Result<Plan, String> {
        if self.iterations == 0 {
            return Err("--iterations must be greater than zero".into());
        }

        let modes = if self.mode.is_empty() {
            Mode::ALL.to_vec()
        } else {
            self.mode.clone()
        };

        let sizes = if self.size.is_empty() {
            power_of_two_sizes(self.min_size, self.max_size)?
        } else {
            let mut sizes = self.size.clone();
            sizes.sort_unstable();
            sizes.dedup();
            if sizes.first() == Some(&0) {
                return Err("--size must be greater than zero".into());
            }
            sizes
        };

        Ok(Plan { modes, sizes })
    }
}

/// Powers of two from the first one at or above `min` up to the last one at or below `max`.
/// Non-power-of-two bounds are rounded inwards, so `4000..=100000` sweeps 4096..=65536.
fn power_of_two_sizes(min: usize, max: usize) -> Result<Vec<usize>, String> {
    if min == 0 {
        return Err("--min-size must be greater than zero".into());
    }
    if max < min {
        return Err("--max-size must not be smaller than --min-size".into());
    }

    let mut sizes = Vec::new();
    let mut size = min.next_power_of_two();
    while size <= max {
        sizes.push(size);
        match size.checked_mul(2) {
            Some(next) => size = next,
            None => break,
        }
    }

    if sizes.is_empty() {
        return Err(format!(
            "no power-of-two message size lies between {min} and {max}"
        ));
    }
    Ok(sizes)
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

impl Mode {
    /// Every mode, in the order a suite runs them.
    pub const ALL: [Mode; 3] = [Mode::Bandwidth, Mode::Latency, Mode::Accuracy];

    pub fn name(&self) -> &'static str {
        match self {
            Mode::Bandwidth => "bandwidth",
            Mode::Latency => "latency",
            Mode::Accuracy => "accuracy",
        }
    }

    /// Smallest message size this mode can be run with, so a sweep can skip the sizes a mode
    /// would only fail on.
    pub fn min_msg_size(&self) -> usize {
        match self {
            // The sequence-number header a received message is identified by.
            Mode::Accuracy => 8,
            _ => 1,
        }
    }
}
