use super::Role;
use crate::comm::Conn;
use crate::error::Result;
use ibverbs::{CompletionQueue, ProtectionDomain, QueuePair};

pub fn run(
    _pd: &ProtectionDomain,
    _cq: &CompletionQueue,
    _qp: &mut QueuePair,
    _conn: &mut Conn,
    _role: Role,
    _msg_size: usize,
    _iterations: usize,
    _tx_depth: usize,
) -> Result<()> {
    unimplemented!("accuracy mode not yet implemented")
}
