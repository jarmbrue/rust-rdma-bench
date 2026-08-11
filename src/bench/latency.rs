//! Send/receive latency over a ping-pong exchange.
//!
//! The client sends a message and waits for the server to echo one back; one iteration is one
//! full round trip. Only the client times anything — the server just turns messages around as
//! fast as it can. Reported figures are **half** round trips (`rtt / 2`), i.e. the one-way
//! latency, matching what perftest's `ib_send_lat` prints.
//!
//! This is deliberately stop-and-wait: exactly one message is in flight in each direction at any
//! time, so `--tx-depth` does not apply here and is ignored.
//!
//! Over UC either half of a round trip can vanish without a trace, which would otherwise leave
//! both sides waiting forever, so each wait is bounded by `bench::IDLE_TIMEOUT`. A round trip that
//! hits the bound is counted as lost and contributes no timing sample. Note that a very late echo
//! can still be picked up by a subsequent iteration's wait and time that one too short; with loss
//! rare enough for the latency figures to mean anything, so is this.

use super::{IDLE_TIMEOUT, Role, completion_error};
use crate::comm::Conn;
use crate::error::Result;
use crate::report::{LatencyStats, Report};
use ibverbs::{CompletionQueue, MemoryRegion, ProtectionDomain, QueuePair, ibv_wc};
use std::time::Instant;

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
) -> Result<Report> {
    if iterations == 0 {
        return Err("latency benchmark needs at least one iteration".into());
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
fn wait_for(cq: &CompletionQueue, wc: &mut [ibv_wc], want: u64) -> Result<u64> {
    let mut pending = want;
    let deadline = Instant::now() + IDLE_TIMEOUT;
    while pending != 0 {
        for c in cq.poll(wc)?.iter() {
            completion_error(c)?;
            pending &= !c.wr_id();
        }
        if pending != 0 && Instant::now() >= deadline {
            break;
        }
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

    // The receive for the first echo has to be posted before the first send goes out.
    unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
    conn.sync()?; // both sides have a receive posted

    for i in 0..iterations {
        let t0 = Instant::now();
        unsafe { qp.post_send(send_mr, .., WR_SEND)? };
        // A timeout means a full IDLE_TIMEOUT of silence, which on any working link means the
        // echoing side has stopped rather than that this one message was unlucky — and the server
        // stops on the same condition. Pushing on would just time out every remaining iteration
        // in turn, so both sides give up together and the rest is reported as skipped.
        if wait_for(cq, &mut wc, WR_SEND | WR_RECV)? != 0 {
            println!(
                "round trip {} timed out after {:?}; {} of {} iterations skipped",
                i,
                IDLE_TIMEOUT,
                iterations - i,
                iterations
            );
            break;
        }
        samples.push(t0.elapsed().as_secs_f64() * 1e6 / 2.0); // µs, half round trip

        // Repost before the next send, so the next echo can never arrive without a receive
        // waiting for it.
        if i + 1 < iterations {
            unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
        }
    }

    conn.sync()?; // both sides done
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

    unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
    conn.sync()?; // both sides have a receive posted

    for _ in 0..iterations {
        // A ping that never arrives means the client has either given up on that iteration or
        // finished the run; either way there is nothing left to echo.
        if wait_for(cq, &mut wc, WR_RECV)? != 0 {
            break;
        }
        // Repost ahead of the echo: the client's next ping follows immediately on the echo, and
        // there must already be a receive queued for it. Doing so on the last iteration too just
        // leaves one unused work request behind on a queue pair that is about to be torn down.
        unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
        unsafe { qp.post_send(send_mr, .., WR_SEND)? };
        wait_for(cq, &mut wc, WR_SEND)?;
        echoed += 1;
    }

    conn.sync()?; // both sides done
    println!("echoed {echoed} of {iterations} messages");
    Ok(Report::Peer)
}
