use super::Role;
use crate::comm::Conn;
use crate::error::Result;
use ibverbs::{CompletionQueue, MemoryRegion, QueuePair};

pub fn run(
    _cq: &CompletionQueue,
    _qp: &mut QueuePair,
    _mr: &mut MemoryRegion<u8>,
    _conn: &mut Conn,
    _role: Role,
    _msg_size: usize,
    _iterations: usize,
    _tx_depth: usize,
) -> Result<()> {
    unimplemented!("latency mode not yet implemented")
}
