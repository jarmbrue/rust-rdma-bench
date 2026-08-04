use crate::bench::{self, Role};
use crate::cli::ServerArgs;
use crate::comm::{self, BenchmarkRequest, ClientEndpoint, Conn, HandshakeAck};
use crate::device;
use crate::error::Result;
use crate::transport;
use ibverbs::{Context, ProtectionDomain};

pub fn run(args: ServerArgs) -> Result<()> {
    let ctx = device::open(args.device.as_deref())?;
    let pd = ctx.alloc_pd()?;
    let listener = comm::listen(args.port)?;

    println!("listening on port {}", args.port);
    loop {
        let mut conn = comm::accept_one(&listener)?;
        if let Err(e) = handle_connection(&ctx, &pd, &mut conn) {
            eprintln!("connection error: {e}");
        }

        if !args.listen {
            break;
        }
        println!("waiting for next connection...");
    }

    Ok(())
}

fn handle_connection(ctx: &Context, pd: &ProtectionDomain, conn: &mut Conn) -> Result<()> {
    let req: BenchmarkRequest = conn.recv_json()?;
    println!("benchmark request: {req:?}");

    if !bench::supported(req.transport, req.mode) {
        let reason = format!("{:?}/{:?} is not implemented yet", req.transport, req.mode);
        conn.send_json(&HandshakeAck::Unsupported(reason))?;
        return Ok(());
    }

    let cq = ctx.create_cq((2 * req.tx_depth) as i32, 0)?;
    let prepared = transport::build(req.transport, pd, &cq, req.tx_depth)?;
    let local_endpoint = prepared.endpoint();
    conn.send_json(&HandshakeAck::Ok {
        endpoint: local_endpoint,
    })?;

    let ClientEndpoint {
        endpoint: remote_endpoint,
    } = conn.recv_json()?;
    let mut qp = prepared.handshake(remote_endpoint)?;
    let mut mr = pd.allocate::<u8>(req.msg_size)?;

    bench::run(
        req.mode,
        &cq,
        &mut qp,
        &mut mr,
        conn,
        Role::Server,
        req.msg_size,
        req.iterations,
        req.tx_depth,
    )
}
