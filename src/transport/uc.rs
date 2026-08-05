//! UC transport.
//!
//! Building the queue pair is the same shape as RC — `QueuePairBuilder` already gates every
//! RC-only attribute on `qp_type`, so `handshake()` moves a UC queue pair through INIT/RTR/RTS
//! with exactly the attribute mask UC requires (no timeout, retry count, RNR timer or atomic
//! depth, which are all RC-only and would be rejected).
//!
//! What differs is the traffic: UC acknowledges nothing and retransmits nothing, so a message the
//! fabric drops is gone silently, with no completion on either side to mark it. Every benchmark
//! that waits on a peer therefore has to bound that wait — see `bench::IDLE_TIMEOUT`.

use crate::error::Result;
use ibverbs::{CompletionQueue, PreparedQueuePair, ProtectionDomain};

/// Builds a UC queue pair with the given send/receive depth, ready to be handed a remote endpoint
/// via `PreparedQueuePair::handshake`.
pub fn build<'res>(
    pd: &'res ProtectionDomain<'res>,
    cq: &'res CompletionQueue<'res>,
    tx_depth: usize,
) -> Result<PreparedQueuePair<'res>> {
    let prepared = pd
        .create_qp(cq, cq, ibverbs::ibv_qp_type::IBV_QPT_UC)
        .set_max_send_wr(tx_depth as u32)
        .set_max_recv_wr(tx_depth as u32)
        // See the matching comment in `transport/rc.rs`: D3OS always reports a GID, so the local
        // side needs a gid_index configured to interoperate.
        .set_gid_index(0)
        .build()?;
    Ok(prepared)
}
