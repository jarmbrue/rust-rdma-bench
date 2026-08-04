use crate::error::Result;
use ibverbs::{CompletionQueue, PreparedQueuePair, ProtectionDomain};

/// Builds an RC queue pair with the given send/receive depth, ready to be handed a remote
/// endpoint via `PreparedQueuePair::handshake`.
pub fn build<'res>(
    pd: &'res ProtectionDomain<'res>,
    cq: &'res CompletionQueue<'res>,
    tx_depth: usize,
) -> Result<PreparedQueuePair<'res>> {
    let prepared = pd
        .create_qp(cq, cq, ibverbs::ibv_qp_type::IBV_QPT_RC)
        .set_max_send_wr(tx_depth as u32)
        .set_max_recv_wr(tx_depth as u32)
        .build()?;
    Ok(prepared)
}
