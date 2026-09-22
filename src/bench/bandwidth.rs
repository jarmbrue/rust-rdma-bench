use super::{Role, WARMUP_ITERS, completion_error};
use crate::comm::Conn;
use crate::error::Result;
use crate::report::{BandwidthStats, Report};
use ibverbs::{CompletionQueue, MemoryRegion, ProtectionDomain, QueuePair, ibv_wc};
use std::time::Instant;

pub fn run(
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
    // One buffer reused for every work request: this only measures throughput, so the messages
    // don't need distinct payloads.
    let mut mr = pd.allocate::<u8>(msg_size)?;

    match role {
        Role::Server => receive(cq, qp, &mut mr, conn, iterations, tx_depth, rx_depth),
        Role::Client => send(cq, qp, &mut mr, conn, msg_size, iterations, tx_depth, rx_depth),
    }
}

/// How many warm-up messages to exchange, discarded, before the timed region starts. Bounded by
/// both queue depths (not just `WARMUP_ITERS`/`iterations`) so a caller-requested depth smaller
/// than `WARMUP_ITERS` can't overflow the send/receive queue when the warm-up batch is posted;
/// both sides compute this identically and independently, so the counts always agree without
/// needing to negotiate it over the wire.
fn warmup_count(iterations: usize, tx_depth: usize, rx_depth: usize) -> usize {
    WARMUP_ITERS.min(iterations).min(tx_depth).min(rx_depth)
}

/// Polls until `n` completions have been seen, discarding them (after checking for errors).
fn drain_n(cq: &CompletionQueue, wc: &mut [ibv_wc], n: usize) -> Result<()> {
    let mut remaining = n;
    while remaining > 0 {
        let completions = cq.poll(wc)?;
        for c in completions.iter() {
            completion_error(c)?;
        }
        remaining -= completions.len().min(remaining);
    }
    Ok(())
}

fn receive(
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    iterations: usize,
    tx_depth: usize,
    rx_depth: usize,
) -> Result<Report> {
    let mut wc = vec![ibv_wc::default(); rx_depth.max(1)];

    // Warm-up: a freshly-RTS queue pair has a one-time settling cost (observed, on this driver/
    // HCA combination, to be large enough to dominate a short run's *entire* measured throughput
    // rather than just its first sample) that has nothing to do with steady-state throughput.
    // Exchanging and discarding a small batch of real messages first absorbs that cost before the
    // clock starts. See `bandwidth::send`'s matching warm-up for the sender side.
    let warmup = warmup_count(iterations, tx_depth, rx_depth);
    for i in 0..warmup {
        unsafe { qp.post_receive(mr, .., i as u64)? };
    }
    conn.sync("bandwidth/receiver: warmup receives posted")?;
    drain_n(cq, &mut wc, warmup)?;
    conn.sync("bandwidth/receiver: warmup done")?;

    let window = rx_depth.min(iterations);
    for i in 0..window {
        unsafe { qp.post_receive(mr, .., i as u64)? };
    }
    let mut posted = window;
    let mut completed = 0usize;

    conn.sync("bandwidth/receiver: receives posted")?;
    // On UC a dropped message produces no completion at all, so this cannot wait for a fixed
    // count — it drains until the stream goes quiet and reports the shortfall.
    let mut last_progress = Instant::now();
    while completed < iterations {
        let completions = cq.poll(&mut wc)?;
        let n = completions.len();
        for c in completions.iter() {
            completion_error(c)?;
        }
        if n == 0 {
            if last_progress.elapsed() >= super::IDLE_TIMEOUT {
                break;
            }
            continue;
        }
        last_progress = Instant::now();
        completed += n;
        for _ in 0..n {
            if posted < iterations {
                unsafe { qp.post_receive(mr, .., posted as u64)? };
                posted += 1;
            }
        }
    }
    conn.sync("bandwidth/receiver: drained")?;
    if completed < iterations {
        println!(
            "received {completed} of {iterations} messages ({} never arrived)",
            iterations - completed
        );
    }
    Ok(Report::Peer)
}

fn send(
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    msg_size: usize,
    iterations: usize,
    tx_depth: usize,
    rx_depth: usize,
) -> Result<Report> {
    let mut wc = vec![ibv_wc::default(); tx_depth.max(1)];

    // Warm-up: see receive()'s matching comment.
    let warmup = warmup_count(iterations, tx_depth, rx_depth);
    conn.sync("bandwidth/sender: waiting for warmup receives posted")?;
    for i in 0..warmup {
        unsafe { qp.post_send(mr, .., i as u64)? };
    }
    drain_n(cq, &mut wc, warmup)?;
    conn.sync("bandwidth/sender: warmup done")?;

    conn.sync("bandwidth/sender: waiting for receives posted")?;
    let t0 = Instant::now();

    let window = tx_depth.min(iterations);
    for i in 0..window {
        unsafe { qp.post_send(mr, .., i as u64)? };
    }
    let mut posted = window;
    let mut completed = 0usize;
    while completed < iterations {
        let completions = cq.poll(&mut wc)?;
        let n = completions.len();
        for c in completions.iter() {
            completion_error(c)?;
        }
        completed += n;
        for _ in 0..n {
            if posted < iterations {
                unsafe { qp.post_send(mr, .., posted as u64)? };
                posted += 1;
            }
        }
    }
    let elapsed = t0.elapsed();
    conn.sync("bandwidth/sender: all sends completed")?;

    Ok(Report::Bandwidth(BandwidthStats {
        msg_size,
        iterations,
        tx_depth,
        elapsed_us: elapsed.as_micros(),
    }))
}
