//! UC transport: not yet implemented.
//!
//! `QueuePairBuilder::build`/`handshake` in rust-ibverbs 0.8.1 support UC in principle (the
//! RC/UC-gated setters in `ibverbs::QueuePairBuilder` already branch on `IBV_QPT_UC`), but the
//! benchmark-side plumbing (transport dispatch, wire protocol) hasn't been wired up for it yet.

use crate::error::Result;
use ibverbs::{CompletionQueue, PreparedQueuePair, ProtectionDomain};

pub fn build<'res>(
    _pd: &'res ProtectionDomain<'res>,
    _cq: &'res CompletionQueue<'res>,
    _tx_depth: usize,
) -> Result<PreparedQueuePair<'res>> {
    unimplemented!("UC transport not yet implemented")
}
