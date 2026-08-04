use crate::bench::{self, Role};
use crate::cli::ClientArgs;
use crate::comm::{self, BenchmarkRequest, ClientEndpoint, HandshakeAck};
use crate::device;
use crate::error::Result;
use crate::transport;

pub fn run(args: ClientArgs) -> Result<()> {
    let ctx = device::open(args.device.as_deref())?;
    let pd = ctx.alloc_pd()?;
    let cq = ctx.create_cq((2 * args.tx_depth) as i32, 0)?;

    let prepared = transport::build(args.transport, &pd, &cq, args.tx_depth)?;
    let local_endpoint = prepared.endpoint();

    let mut conn = comm::connect(&args.host, args.port)?;
    conn.send_json(&BenchmarkRequest {
        transport: args.transport,
        mode: args.mode,
        msg_size: args.size,
        iterations: args.iterations,
        tx_depth: args.tx_depth,
    })?;

    let ack: HandshakeAck = conn.recv_json()?;
    let remote_endpoint = match ack {
        HandshakeAck::Unsupported(reason) => {
            return Err(format!("server rejected benchmark request: {reason}").into());
        }
        HandshakeAck::Ok { endpoint } => endpoint,
    };

    let mut qp = prepared.handshake(remote_endpoint)?;
    conn.send_json(&ClientEndpoint {
        endpoint: local_endpoint,
    })?;

    let mut mr = pd.allocate::<u8>(args.size)?;

    bench::run(
        args.mode,
        &cq,
        &mut qp,
        &mut mr,
        &mut conn,
        Role::Client,
        args.size,
        args.iterations,
        args.tx_depth,
    )
}
