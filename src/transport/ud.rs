//! UD transport: deliberately not implemented.
//!
//! rust-ibverbs 0.8.1 has no real UD support: `PreparedQueuePair::handshake` unconditionally
//! applies RC/UC-style `dest_qp_num`/`ah_attr` logic even when `qp_type` is UD (the crate's own
//! comment on that code path reads "TODO: this is only valid for RC and UC" while doing it
//! anyway), there is no `AddressHandle`/`ibv_create_ah` wrapper, and `post_send` has no per-send
//! address-handle parameter. Building UD support here would mean bypassing `handshake()` entirely
//! and adding AH/UD send support, either upstream in rust-ibverbs or via raw `ffi::` calls from
//! this crate — out of scope for now.

use crate::error::Result;
use ibverbs::{CompletionQueue, PreparedQueuePair, ProtectionDomain};

pub fn build<'res>(
    _pd: &'res ProtectionDomain<'res>,
    _cq: &'res CompletionQueue<'res>,
    _tx_depth: usize,
) -> Result<PreparedQueuePair<'res>> {
    unimplemented!("UD transport not yet implemented (see module doc comment)")
}
