pub mod accuracy;
pub mod bandwidth;
pub mod latency;
pub mod rdma;

use crate::cli::{Mode, Transport};
use crate::comm::Conn;
use crate::report::Report;
use ibverbs::{CompletionQueue, ProtectionDomain, QueuePair, ibv_wc};
use std::io::{Error, Result};
use std::time::Duration;

/// How long a poll loop keeps spinning without seeing a completion before it gives up and treats
/// whatever it was waiting for as lost.
///
/// Unreliable transports drop messages silently: nothing tells the receiver that a message it is
/// waiting for is never coming, so any loop that waits on the peer needs a backstop or a single
/// dropped message hangs the run. Deliberately far longer than any real inter-message gap, so it
/// only ever fires on actual loss (or, on RC, on a hang that would otherwise be permanent).
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a bandwidth/latency run pauses, synchronized on both sides, right after the
/// handshake before starting its timed region.
///
/// A freshly-RTS queue pair has a one-time settling cost that, on the ib1/ib2 ConnectX-3
/// hardware, was observed to swamp a short run's *entire* measured throughput rather than just
/// its first sample (a bandwidth run with no warm-up looked ~8-15x slower and showed zero benefit
/// from `--tx-depth`, both symptoms of this dominating instead of real steady-state behavior).
/// This is a genuinely time-bound cost, not a "number of messages" one: a discarded warm-up batch
/// bounded by queue depth (a few dozen to a few hundred messages) finishes in well under a
/// millisecond even at the slow cold rate, nowhere near enough elapsed time to matter — only an
/// actual pause of this rough magnitude fixed it in testing. `rust-perftest`'s client sleeps
/// 100ms before starting its own timer for the same reason.
pub const WARMUP_SETTLE: Duration = Duration::from_millis(100);

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
    match (transport, mode) {
        (Transport::Ud, _) => false,
        // RDMA READ is not in UC's transport-service repertoire (IBTA 1.2.1, table 44) — UC has
        // RDMA WRITE but no read/atomics.
        (Transport::Uc, Mode::RdmaRead) => false,
        // Every other mode works over both connected transports: the SEND/RECV benchmarks are
        // written to tolerate the loss UC allows, so none of them needs RC's guarantees, and RDMA
        // WRITE degrades the same way SEND does on UC (no ack, no retransmit).
        (Transport::Rc | Transport::Uc, _) => true,
    }
}

/// Turns a failed work completion into an error; successful ones pass through.
pub fn completion_error(wc: &ibv_wc) -> Result<()> {
    if let Some((status, vendor_err)) = wc.error() {
        return Err(Error::other(format!(
            "WC error: {status:?} vendor_err={vendor_err}"
        )));
    }
    Ok(())
}

/// Runs the benchmark identified by `mode` over an already-handshaked queue pair and hands back
/// what it measured. Nothing here prints: the caller decides whether the numbers become a
/// standalone table or one row of a sweep (see `crate::report`).
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
    rx_depth: usize,
) -> Result<Report> {
    match mode {
        Mode::Bandwidth => bandwidth::run(
            pd, cq, qp, conn, role, msg_size, iterations, tx_depth, rx_depth,
        ),
        Mode::Latency => latency::run(
            pd, cq, qp, conn, role, msg_size, iterations, tx_depth, rx_depth,
        ),
        Mode::Accuracy => accuracy::run(
            pd, cq, qp, conn, role, msg_size, iterations, tx_depth, rx_depth,
        ),
        Mode::RdmaWrite | Mode::RdmaRead => rdma::run(
            pd, cq, qp, conn, role, msg_size, iterations, tx_depth, rx_depth,
        ),
    }
}
