//! Delivery and integrity accuracy of a one-way send stream.
//!
//! The client streams `iterations` messages, each carrying a payload that is a deterministic
//! function of its sequence number. The server checks every message it receives against the
//! payload that sequence number should have had, and reports what fraction of the bytes and bits
//! the client sent actually arrived intact.
//!
//! Over RC this should always come out at 100% — the transport itself guarantees it, so the RC
//! run is really a self-check of the harness. The mode exists for UC and UD, where messages can
//! be dropped, duplicated or reordered and nothing below the benchmark notices.
//!
//! Anything the client sent but the server never saw counts as fully incorrect, so the reported
//! accuracy covers loss as well as corruption. Because loss is the expected case here, the server
//! cannot wait for a fixed number of messages: it stops once no completion has arrived for
//! `bench::IDLE_TIMEOUT` and treats whatever is still missing as lost.

use super::{IDLE_TIMEOUT, Role, completion_error};
use crate::comm::{AccuracyReport, Conn};
use crate::error::Result;
use crate::report::Report;
use ibverbs::{CompletionQueue, MemoryRegion, ProtectionDomain, QueuePair, ibv_wc};
use std::ops::Range;
use std::time::Instant;

/// Bytes at the front of every message holding its sequence number, little endian. The receiver
/// reads it to know which message it is looking at, since messages can arrive out of order or not
/// at all.
const HEADER_LEN: usize = 8;

pub fn run(
    pd: &ProtectionDomain,
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    conn: &mut Conn,
    role: Role,
    msg_size: usize,
    iterations: usize,
    tx_depth: usize,
) -> Result<Report> {
    if iterations == 0 {
        return Err("accuracy benchmark needs at least one iteration".into());
    }
    if msg_size < HEADER_LEN {
        return Err(format!(
            "accuracy benchmark needs a message size of at least {HEADER_LEN} bytes for the \
             sequence-number header"
        )
        .into());
    }

    // Every message in flight needs its own buffer: unlike the bandwidth mode the payloads differ
    // per message, so a slot must not be rewritten until its work request has completed. They all
    // live in one memory region, addressed by sub-ranges, so registration happens once.
    let window = tx_depth.max(1).min(iterations);
    let mut mr = pd.allocate::<u8>(window * msg_size)?;

    match role {
        Role::Client => send(cq, qp, &mut mr, conn, msg_size, iterations, window),
        Role::Server => receive(cq, qp, &mut mr, conn, msg_size, iterations, window),
    }
}

/// Byte range of `slot` within the windowed memory region.
fn slot_range(slot: usize, msg_size: usize) -> Range<usize> {
    let start = slot * msg_size;
    start..start + msg_size
}

/// One step of splitmix64, used to generate payloads that are cheap to produce, identical on both
/// sides, and varied enough that a corrupted message is very unlikely to still match.
fn next_word(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Writes the payload message `seq` must carry — its sequence number, then a stream derived from
/// it — into a buffer exactly one message long.
fn fill_payload(buf: &mut [u8], seq: u64) {
    buf[..HEADER_LEN].copy_from_slice(&seq.to_le_bytes());
    let mut state = seq;
    for chunk in buf[HEADER_LEN..].chunks_mut(8) {
        let word = next_word(&mut state).to_le_bytes();
        chunk.copy_from_slice(&word[..chunk.len()]);
    }
}

/// Client side: streams every message once, keeping `window` of them in flight.
fn send(
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    msg_size: usize,
    iterations: usize,
    window: usize,
) -> Result<Report> {
    let mut wc = vec![ibv_wc::default(); window];

    conn.sync()?; // wait until the receiver has its receives posted

    for seq in 0..window {
        fill_payload(&mut mr[slot_range(seq, msg_size)], seq as u64);
        unsafe { qp.post_send(mr, slot_range(seq, msg_size), seq as u64)? };
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
        // The send queue completes in order, so after `completed` completions every slot except
        // the `posted - completed` still outstanding is free to be refilled.
        for _ in 0..n {
            if posted < iterations {
                let slot = posted % window;
                fill_payload(&mut mr[slot_range(slot, msg_size)], posted as u64);
                unsafe { qp.post_send(mr, slot_range(slot, msg_size), posted as u64)? };
                posted += 1;
            }
        }
    }

    conn.sync()?; // "everything I was going to send has left the queue"
    let report: AccuracyReport = conn.recv_msg()?;
    Ok(Report::Accuracy(report))
}

/// Server side: checks every arriving message against what its sequence number should have held.
fn receive(
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    msg_size: usize,
    iterations: usize,
    window: usize,
) -> Result<Report> {
    let mut wc = vec![ibv_wc::default(); window];
    for slot in 0..window {
        unsafe { qp.post_receive(mr, slot_range(slot, msg_size), slot as u64)? };
    }
    conn.sync()?; // "receives are posted, go ahead"

    let mut expected = vec![0u8; msg_size];
    let mut seen = vec![false; iterations];
    let mut report = AccuracyReport {
        msg_size,
        sent: iterations,
        ..AccuracyReport::default()
    };

    let mut last_progress = Instant::now();
    // Completions are copied out of `wc` so its borrow ends before the buffers are touched.
    let mut batch: Vec<(usize, usize)> = Vec::with_capacity(window);
    while report.received < iterations {
        batch.clear();
        for c in cq.poll(&mut wc)?.iter() {
            completion_error(c)?;
            batch.push((c.wr_id() as usize, c.len()));
        }
        if batch.is_empty() {
            if last_progress.elapsed() >= IDLE_TIMEOUT {
                break;
            }
            continue;
        }
        last_progress = Instant::now();
        report.received += batch.len();

        for &(slot, len) in &batch {
            let range = slot_range(slot, msg_size);
            let got = &mr[range.start..range.start + len.min(msg_size)];
            check(got, &mut expected, msg_size, &mut seen, &mut report);
            unsafe { qp.post_receive(mr, range, slot as u64)? };
        }
    }
    report.lost = seen.iter().filter(|s| !**s).count();

    conn.sync()?; // meets the sender's post-send barrier
    conn.send_msg(&report)?;
    Ok(Report::Peer)
}

/// Attributes one received message to a sequence number and folds it into `report`. `got` is the
/// prefix that actually arrived, which is the whole message unless it was truncated; `expected`
/// is scratch space one message long.
fn check(
    got: &[u8],
    expected: &mut [u8],
    msg_size: usize,
    seen: &mut [bool],
    report: &mut AccuracyReport,
) {
    if got.len() < HEADER_LEN {
        report.unidentifiable += 1;
        return;
    }
    let seq = u64::from_le_bytes(got[..HEADER_LEN].try_into().expect("checked above"));
    // A sequence number from outside this run means the header itself was corrupted (or the
    // message is a stray from elsewhere); there is nothing to compare it against.
    let Ok(seq_idx) = usize::try_from(seq) else {
        report.unidentifiable += 1;
        return;
    };
    if seq_idx >= seen.len() {
        report.unidentifiable += 1;
        return;
    }
    if seen[seq_idx] {
        // Counting a duplicate's bytes again would let the correct totals exceed what was sent.
        report.duplicated += 1;
        return;
    }
    seen[seq_idx] = true;

    if got.len() < msg_size {
        report.truncated += 1;
    }
    fill_payload(expected, seq);
    // Bytes beyond `got` never arrived, so they simply never contribute to the correct totals.
    let mut correct_bytes = 0u64;
    let mut correct_bits = 0u64;
    for (a, b) in got.iter().zip(expected.iter()) {
        let diff = a ^ b;
        if diff == 0 {
            correct_bytes += 1;
        }
        correct_bits += u64::from(8 - diff.count_ones());
    }
    if correct_bytes as usize != msg_size {
        report.corrupted += 1;
    }
    report.correct_bytes += correct_bytes;
    report.correct_bits += correct_bits;
}
