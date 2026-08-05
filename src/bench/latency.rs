//! Send/receive latency over a ping-pong exchange.
//!
//! The client sends a message and waits for the server to echo one back; one iteration is one
//! full round trip. Only the client times anything — the server just turns messages around as
//! fast as it can. Reported figures are **half** round trips (`rtt / 2`), i.e. the one-way
//! latency, matching what perftest's `ib_send_lat` prints.
//!
//! This is deliberately stop-and-wait: exactly one message is in flight in each direction at any
//! time, so `--tx-depth` does not apply here and is ignored.

use super::{Role, completion_error};
use crate::comm::Conn;
use crate::error::Result;
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
) -> Result<()> {
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
/// `WR_RECV`). Busy-polls without blocking, which is the point: a completion-channel wakeup would
/// add far more delay than the latency being measured.
fn wait_for(cq: &CompletionQueue, wc: &mut [ibv_wc], want: u64) -> Result<()> {
    let mut pending = want;
    while pending != 0 {
        for c in cq.poll(wc)?.iter() {
            completion_error(c)?;
            pending &= !c.wr_id();
        }
    }
    Ok(())
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
) -> Result<()> {
    let mut wc = [ibv_wc::default(); 2];
    let mut samples = Vec::with_capacity(iterations);

    // The receive for the first echo has to be posted before the first send goes out.
    unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
    conn.sync()?; // both sides have a receive posted

    for i in 0..iterations {
        let t0 = Instant::now();
        unsafe { qp.post_send(send_mr, .., WR_SEND)? };
        wait_for(cq, &mut wc, WR_SEND | WR_RECV)?;
        samples.push(t0.elapsed().as_secs_f64() * 1e6 / 2.0); // µs, half round trip

        // Repost before the next send, so the next echo can never arrive without a receive
        // waiting for it.
        if i + 1 < iterations {
            unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
        }
    }

    conn.sync()?; // both sides done
    report(&mut samples, msg_size);
    Ok(())
}

/// Server side: echoes every message back, untimed.
fn pong(
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    send_mr: &mut MemoryRegion<u8>,
    recv_mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    iterations: usize,
) -> Result<()> {
    let mut wc = [ibv_wc::default(); 2];

    unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
    conn.sync()?; // both sides have a receive posted

    for i in 0..iterations {
        wait_for(cq, &mut wc, WR_RECV)?;
        // Repost ahead of the echo: the client's next ping follows immediately on the echo, and
        // there must already be a receive queued for it.
        if i + 1 < iterations {
            unsafe { qp.post_receive(recv_mr, .., WR_RECV)? };
        }
        unsafe { qp.post_send(send_mr, .., WR_SEND)? };
        wait_for(cq, &mut wc, WR_SEND)?;
    }

    conn.sync()?; // both sides done
    println!("echoed {iterations} messages");
    Ok(())
}

/// Sorts `samples` in place and prints the usual latency summary. All figures are µs.
fn report(samples: &mut [f64], msg_size: usize) {
    samples.sort_by(f64::total_cmp);
    let n = samples.len();

    let avg = samples.iter().sum::<f64>() / n as f64;
    let stdev = if n > 1 {
        (samples.iter().map(|s| (s - avg).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
    } else {
        0.0
    };
    // Nearest-rank percentiles over the sorted samples.
    let percentile = |p: f64| samples[(((n as f64) * p).ceil() as usize).clamp(1, n) - 1];

    println!(
        "{:>8}  {:>12}  {:>12}  {:>12}  {:>16}  {:>12}  {:>14}  {:>16}  {:>18}",
        "#bytes",
        "#iterations",
        "t_min[usec]",
        "t_max[usec]",
        "t_typical[usec]",
        "t_avg[usec]",
        "t_stdev[usec]",
        "99%[usec]",
        "99.9%[usec]",
    );
    println!(
        "{:>8}  {:>12}  {:>12.2}  {:>12.2}  {:>16.2}  {:>12.2}  {:>14.2}  {:>16.2}  {:>18.2}",
        msg_size,
        n,
        samples[0],
        samples[n - 1],
        percentile(0.50),
        avg,
        stdev,
        percentile(0.99),
        percentile(0.999),
    );
}
