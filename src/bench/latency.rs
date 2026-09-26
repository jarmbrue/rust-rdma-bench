//! Send/receive latency over a ping-pong exchange.
//!
//! The client sends a message and waits for the server to echo one back; one iteration is one
//! full round trip. Only the client times anything — the server just turns messages around as
//! fast as it can. Reported figures are **half** round trips (`rtt / 2`), i.e. the one-way
//! latency, matching what perftest's `ib_send_lat` prints.
//!
//! This is deliberately stop-and-wait: exactly one message is in flight in each direction at any
//! time, so neither `--tx-depth` nor `--rx-depth` applies here and both are ignored.
//!
//! Over UC either half of a round trip can vanish without a trace, which would otherwise leave
//! both sides waiting forever, so each wait is bounded by `bench::IDLE_TIMEOUT`. A round trip that
//! hits the bound is counted as lost and contributes no timing sample. Note that a very late echo
//! can still be picked up by a subsequent iteration's wait and time that one too short; with loss
//! rare enough for the latency figures to mean anything, so is this.

use super::{IDLE_TIMEOUT, Role, WARMUP_SETTLE, completion_error};
use crate::comm::Conn;
use crate::report::{LatencyStats, Report};
use ibverbs::{CompletionQueue, MemoryRegion, ProtectionDomain, QueuePair, ibv_wc};
use std::io::{Error, ErrorKind, Result};
use std::time::{Duration, Instant};

/// `wr_id`s for the two work requests a side can have outstanding. They're powers of two so
/// `wait_for` can track what it is still waiting on as a bitmask.
const WR_SEND: u64 = 1;
const WR_RECV: u64 = 2;

pub fn run(
    pd: &ProtectionDomain,
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    conn: &mut Conn,
    role: Role,
    msg_size: usize,
    iterations: usize,
    _tx_depth: usize,
    _rx_depth: usize,
) -> Result<Report> {
    if iterations == 0 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "latency benchmark needs at least one iteration",
        ));
    }

    // Separate buffers per direction: reusing one region for both would mean the reply lands in
    // the very bytes the outgoing message was read from.
    let mut send_mr = pd.allocate::<u8>(msg_size)?;
    let mut recv_mr = pd.allocate::<u8>(msg_size)?;

    match role {
        Role::Client => ping(
            cq,
            qp,
            &mut send_mr,
            &mut recv_mr,
            conn,
            msg_size,
            iterations,
        ),
        Role::Server => pong(cq, qp, &mut send_mr, &mut recv_mr, conn, iterations),
    }
}

/// Polls until a completion has been seen for every `wr_id` in `want` (a bitmask of `WR_SEND` /
/// `WR_RECV`), or until `IDLE_TIMEOUT` passes with nothing outstanding arriving. Returns the
/// subset of `want` that never showed up, so an empty return means the whole wait was satisfied.
///
/// Busy-polls without blocking, which is the point: a completion-channel wakeup would add far more
/// delay than the latency being measured.
fn wait_for(cq: &CompletionQueue, wc: &mut [ibv_wc], want: u64, timeout: Duration) -> Result<u64> {
    const CHECK_TIMEOUT_ITER: u64 = 100;
    let mut pending = want;
    let deadline = Instant::now() + timeout;
    let mut i: u64 = 0;
    while pending != 0 {
        for c in cq.poll(wc)?.iter() {
            completion_error(c)?;
            pending &= !c.wr_id();
        }

        if i % CHECK_TIMEOUT_ITER == 0 && Instant::now() >= deadline {
            return Err(Error::new(
                ErrorKind::TimedOut,
                format!("poll loop timed out after {}ms", timeout.as_millis()),
            ));
        }

        i += 1;
    }
    Ok(pending)
}

/// Client side: sends a message, waits for the echo, records the round trip.
fn ping(
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    send_mr: &mut MemoryRegion<u8>,
    recv_mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    msg_size: usize,
    iterations: usize,
) -> Result<Report> {
    let mut wc = [ibv_wc::default(); 2];
    let mut samples = Vec::with_capacity(iterations);

    // Warm-up: see `bench::WARMUP_SETTLE`'s doc comment. Without this, the queue pair's one-time
    // settling cost shows up as a single, wildly-outlying first sample here.
    conn.sync("latency/ping: warmup barrier")?;
    std::thread::sleep(WARMUP_SETTLE);
    conn.sync("latency/ping: warmup done")?;

    // The receive for the first echo has to be posted before the first send goes out.
    unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
    unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
    conn.sync("latency/ping: receive posted")?;

    for i in 0..iterations {
        let t0 = Instant::now();
        unsafe { qp.post_send(send_mr, .., WR_SEND)? };
        // A timeout means echoing side has stopped rather than that this one message was unlucky
        if let Err(e) = wait_for(cq, &mut wc, WR_SEND | WR_RECV, IDLE_TIMEOUT) {
            match e.kind() {
                ErrorKind::TimedOut => break,
                _ => return Err(e)
            }
        }
        samples.push(t0.elapsed().as_secs_f64() * 1e6 / 2.0); // µs, half round trip

        if i + 1 < iterations {
            unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
        }
    }

    conn.sync("latency/ping: round trips done")?;
    // A run where every round trip timed out is a result too — an empty one. Reporting it as such
    // rather than as an error keeps one dead size from aborting a whole sweep.
    Ok(Report::Latency(LatencyStats::from_samples(
        msg_size, &samples,
    )))
}

/// Server side: echoes every message back, untimed.
fn pong(
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    send_mr: &mut MemoryRegion<u8>,
    recv_mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    iterations: usize,
) -> Result<Report> {
    let mut wc = [ibv_wc::default(); 2];
    let mut echoed = 0usize;

    conn.sync("latency/pong: warmup barrier")?;
    std::thread::sleep(WARMUP_SETTLE);
    conn.sync("latency/pong: warmup done")?;

    unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
    conn.sync("latency/pong: receive posted")?;

    for _ in 0..iterations {
        wait_for(cq, &mut wc, WR_RECV, IDLE_TIMEOUT)?;
        unsafe { qp.post_send(send_mr, .., WR_SEND)? };
        unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
        echoed += 1;
    }

    conn.sync("latency/pong: echoing done")?;
    if echoed != iterations {
        println!("only echoed {echoed} of {iterations} messages");
    }
    Ok(Report::Peer)
}
