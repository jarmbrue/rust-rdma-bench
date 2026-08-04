use super::Role;
use crate::comm::Conn;
use crate::error::Result;
use ibverbs::{ibv_wc, CompletionQueue, MemoryRegion, QueuePair};
use std::time::Instant;

pub fn run(
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    role: Role,
    msg_size: usize,
    iterations: usize,
    tx_depth: usize,
) -> Result<()> {
    match role {
        Role::Server => receive(cq, qp, mr, conn, iterations, tx_depth),
        Role::Client => send(cq, qp, mr, conn, msg_size, iterations, tx_depth),
    }
}

fn completion_error(wc: &ibv_wc) -> Result<()> {
    if let Some((status, vendor_err)) = wc.error() {
        return Err(format!("WC error: {status:?} vendor_err={vendor_err}").into());
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
) -> Result<()> {
    let window = tx_depth.min(iterations);
    for i in 0..window {
        unsafe { qp.post_receive(mr, .., i as u64)? };
    }
    let mut posted = window;
    let mut completed = 0usize;
    let mut wc = vec![ibv_wc::default(); tx_depth.max(1)];

    conn.sync()?; // "ready"
    while completed < iterations {
        let completions = cq.poll(&mut wc)?;
        let n = completions.len();
        for c in completions.iter() {
            completion_error(c)?;
        }
        completed += n;
        for _ in 0..n {
            if posted < iterations {
                unsafe { qp.post_receive(mr, .., posted as u64)? };
                posted += 1;
            }
        }
    }
    conn.sync()?; // "done draining"
    println!("received {completed} messages");
    Ok(())
}

fn send(
    cq: &CompletionQueue,
    qp: &mut QueuePair,
    mr: &mut MemoryRegion<u8>,
    conn: &mut Conn,
    msg_size: usize,
    iterations: usize,
    tx_depth: usize,
) -> Result<()> {
    let mut wc = vec![ibv_wc::default(); tx_depth.max(1)];

    conn.sync()?; // wait for "ready"
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
    conn.sync()?;

    let secs = elapsed.as_secs_f64();
    let bytes = iterations as f64 * msg_size as f64;
    let bw_gbps = bytes * 8.0 / secs / 1e9;
    let msg_rate_mpps = iterations as f64 / secs / 1e6;
    println!(
        "{:>8}  {:>12}  {:>10}  {:>18}  {:>14}",
        "#bytes", "#iterations", "tx_depth", "BW avg[Gb/sec]", "MsgRate[Mpps]"
    );
    println!(
        "{:>8}  {:>12}  {:>10}  {:>18.2}  {:>14.6}",
        msg_size, iterations, tx_depth, bw_gbps, msg_rate_mpps
    );
    Ok(())
}
