//! Responder half of the one-sided RDMA WRITE/READ bandwidth benchmark whose initiator lives on
//! the D3OS side (see `D3OS/os/application/rdma-bench/src/bench/rdma.rs` for the full picture and
//! the windowing scheme both ends agree on independently, without exchanging it).
//!
//! Only `Role::Server` is implemented here: the crates.io `ibverbs` 0.9.2 this crate is pinned to
//! has no `rdma_write`/`rdma_read` verb, and `QueuePair` keeps its raw `ibv_qp` pointer private, so
//! there is no way for this side to *initiate* an RDMA operation short of forking that crate. The
//! responder needs none of that — it only has to register a buffer and hand out its address and
//! rkey, and `MemoryRegion` already exposes both: the address through its `Deref<Target = [u8]>`
//! (the `Vec` backing it is sized once and never reallocated, so the pointer is stable for the
//! MR's lifetime) and the key through `MemoryRegion::rkey`. Since the responder doesn't care
//! whether the initiator will read or write it, one function serves both `Mode::RdmaWrite` and
//! `Mode::RdmaRead`.

use super::Role;
use crate::comm::{Conn, RemoteBufferInfo, RemoteMemoryRegion};
use crate::error::Result;
use crate::report::Report;
use ibverbs::{CompletionQueue, ProtectionDomain, QueuePair};

pub fn run(
    pd: &ProtectionDomain,
    _cq: &CompletionQueue,
    _qp: &mut QueuePair,
    conn: &mut Conn,
    role: Role,
    msg_size: usize,
    iterations: usize,
    tx_depth: usize,
) -> Result<Report> {
    if role == Role::Client {
        return Err(
            "this build of rust-rdma-bench cannot initiate RDMA WRITE/READ (crates.io \
            ibverbs 0.9.2 has no rdma_write/rdma_read verb); run this mode with D3OS as the \
            client instead"
                .into(),
        );
    }

    // Same formula the D3OS initiator uses to size its own local buffer, derived independently by
    // both ends from parameters already in the `BenchmarkRequest` rather than exchanged.
    let window = tx_depth.max(1).min(iterations);
    let mr = pd.allocate::<u8>(window * msg_size)?;
    let remote = RemoteMemoryRegion {
        addr: mr.as_ptr() as u64,
        len: mr.len(),
        rkey: mr.rkey().key,
        phantom: (),
    };

    conn.send_msg(&RemoteBufferInfo { remote })?;
    conn.sync("rdma/responder: ready")?;
    conn.sync("rdma/responder: initiator done")?;
    Ok(Report::Peer)
}
