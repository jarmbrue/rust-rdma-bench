pub mod accuracy;
pub mod bandwidth;
pub mod latency;

use crate::cli::{Mode, Transport};
use crate::comm::Conn;
use crate::error::Result;
use ibverbs::{CompletionQueue, ProtectionDomain, QueuePair, ibv_wc};

/// Which side of the benchmark connection this process is playing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Client,
    Server,
}

/// Whether `(transport, mode)` has an actual implementation, as opposed to a stub. Checked
/// before any RDMA resources are built for a connection, so an unsupported combination can be
/// rejected cleanly instead of failing partway through setup.
pub fn supported(transport: Transport, mode: Mode) -> bool {
    matches!(
        (transport, mode),
        (Transport::Rc, Mode::Bandwidth)
            | (Transport::Rc, Mode::Latency)
            | (Transport::Rc, Mode::Accuracy)
    )
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
