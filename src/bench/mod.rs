pub mod accuracy;
pub mod bandwidth;
pub mod latency;

use crate::cli::{Mode, Transport};
use crate::comm::Conn;
use crate::error::Result;
use ibverbs::{CompletionQueue, ProtectionDomain, QueuePair, ibv_wc};
use std::time::Duration;

/// How long a poll loop keeps spinning without seeing a completion before it gives up and treats
/// whatever it was waiting for as lost.
///
/// Unreliable transports drop messages silently: nothing tells the receiver that a message it is
/// waiting for is never coming, so any loop that waits on the peer needs a backstop or a single
/// dropped message hangs the run. Deliberately far longer than any real inter-message gap, so it
/// only ever fires on actual loss (or, on RC, on a hang that would otherwise be permanent).
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(2);

/// Which side of the benchmark connection this process is playing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Client,
    Server,
}

/// Whether `(transport, mode)` has an actual implementation, as opposed to a stub. Checked
/// before any RDMA resources are built for a connection, so an unsupported combination can be
/// rejected cleanly instead of failing partway through setup.
pub fn supported(transport: Transport, _mode: Mode) -> bool {
    match transport {
        // Every mode works over both connected transports: the benchmarks are written to tolerate
        // the loss UC allows, so none of them needs RC's guarantees.
        Transport::Rc | Transport::Uc => true,
        Transport::Ud => false,
    }
}

/// Turns a failed work completion into an error; successful ones pass through.
pub fn completion_error(wc: &ibv_wc) -> Result<()> {
    if let Some((status, vendor_err)) = wc.error() {
        return Err(format!("WC error: {status:?} vendor_err={vendor_err}").into());
    }
    Ok(())
}

/// Runs the benchmark identified by `mode` over an already-handshaked queue pair.
///
/// Memory regions are allocated by the individual benchmark rather than by the caller, because
/// how many buffers a run needs is a per-benchmark concern: streaming one direction reuses a
/// single buffer, while a ping-pong needs a separate send and receive buffer per side.
pub fn run(
    mode: Mode,
    pd: &ProtectionDomain,
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    conn: &mut Conn,
    role: Role,
    msg_size: usize,
    iterations: usize,
    tx_depth: usize,
) -> Result<()> {
    match mode {
        Mode::Bandwidth => bandwidth::run(pd, cq, qp, conn, role, msg_size, iterations, tx_depth),
        Mode::Latency => latency::run(pd, cq, qp, conn, role, msg_size, iterations, tx_depth),
        Mode::Accuracy => accuracy::run(pd, cq, qp, conn, role, msg_size, iterations, tx_depth),
    }
}
