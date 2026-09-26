pub mod rc;
pub mod uc;
pub mod ud;

use crate::cli::Transport;
use ibverbs::{CompletionQueue, PreparedQueuePair, ProtectionDomain};
use std::io::Result;

/// Builds a queue pair of the requested transport type, ready to be handshaked with a remote
/// endpoint.
pub fn build<'res>(
    transport: Transport,
    pd: &'res ProtectionDomain<'res>,
    cq: &'res CompletionQueue<'res>,
    tx_depth: usize,
    rx_depth: usize,
) -> Result<PreparedQueuePair<'res>> {
    match transport {
        Transport::Rc => rc::build(pd, cq, tx_depth, rx_depth),
        Transport::Uc => uc::build(pd, cq, tx_depth, rx_depth),
        Transport::Ud => ud::build(pd, cq, tx_depth, rx_depth),
    }
}
