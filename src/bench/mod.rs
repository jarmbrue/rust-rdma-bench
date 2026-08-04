pub mod accuracy;
pub mod bandwidth;
pub mod latency;

use crate::cli::{Mode, Transport};
use crate::comm::Conn;
use crate::error::Result;
use ibverbs::{CompletionQueue, MemoryRegion, QueuePair};

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
    matches!((transport, mode), (Transport::Rc, Mode::Bandwidth))
}

/// Runs the benchmark identified by `mode` over an already-handshaked queue pair.
pub fn run(
    mode: Mode,
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    role: Role,
    msg_size: usize,
    iterations: usize,
    tx_depth: usize,
) -> Result<()> {
    match mode {
        Mode::Bandwidth => bandwidth::run(cq, qp, mr, conn, role, msg_size, iterations, tx_depth),
        Mode::Latency => latency::run(cq, qp, mr, conn, role, msg_size, iterations, tx_depth),
        Mode::Accuracy => accuracy::run(cq, qp, mr, conn, role, msg_size, iterations, tx_depth),
    }
}
