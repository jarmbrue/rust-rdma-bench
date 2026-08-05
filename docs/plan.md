# rust-rdma-bench: initial scaffolding + working RC bandwidth benchmark

## Context

`rust-rdma-bench` is currently just the bare `cargo new` scaffold (no deps, `src/main.rs` prints
"Hello, world!"). It's meant to become the Linux half of a two-sided RDMA benchmark tool for a
master's thesis on the D3OS InfiniBand driver — a future D3OS-side twin will talk to it over real
RDMA. The goal now is to start the app: put in place a module layout that has a clear home for all
three transports (RC/UC/UD) and all three benchmark modes (bandwidth/latency/accuracy), and get
exactly one combination — **RC + bandwidth** — fully working end-to-end between two separate OS
processes, as a proof that the whole pipeline (device setup, OOB handshake, QP handshake, verbs
send/recv, completion polling, throughput reporting) actually works. Everything else is a clearly
labeled stub for later.

`rust-ibverbs` is targeted at version `0.9.2` (D3OS's own RDMA library was updated to mirror this
version too, so the two thesis implementations stay in step). An earlier draft of this plan pinned
`0.8.1` to match D3OS at the time; that version's `ibverbs-sys` build failed on Linux because its
vendored `rdma-core`'s CMake build derives a Unix-domain-socket path (`ibacm`'s) from
`CMAKE_INSTALL_PREFIX`, which `cmake-rs` defaults to Cargo's `OUT_DIR` — a path long enough, nested
under a project's `target/` dir, to exceed `sockaddr_un.sun_path`'s 108-byte limit and fail a
`BUILD_ASSERT`. `0.9.2` fixes this upstream by hardcoding `CMAKE_INSTALL_PREFIX=/usr` during the
(build-only, nothing is actually installed) CMake invocation. The dependency is pulled from
**crates.io** (`ibverbs = "0.9.2"`), not as a path dependency on the local `../rust-ibverbs`
checkout — that local checkout has its own uncommitted local modifications (its vendored
`rdma-core` submodule shows as modified in `git status`) and is meant purely as reading/reference
material for this thesis, not something this crate should build against directly.

**Queue depth / pipelining**: `ProtectionDomain::create_qp` defaults to `max_send_wr=1,
max_recv_wr=1`, but `QueuePairBuilder` exposes public setters — `set_max_send_wr(n)` and
`set_max_recv_wr(n)` (`ibverbs/src/lib.rs:893,901`) — callable on the builder `create_qp(...)`
returns, before `.build()`. So real windowed/pipelined sends are possible and the benchmark loop
below uses them (a `--tx-depth` flag), rather than being forced into stop-and-wait. (Note:
`QueuePairBuilder::new` itself is a private constructor — the only way to get a builder is via
`ProtectionDomain::create_qp`, which is fine since the setters cover what's needed.)

UD is deferred entirely: `handshake()` unconditionally applies RC/UC-style `dest_qp_num`/`ah_attr`
logic even for UD (`lib.rs:1199-1201` in 0.9.2, comment literally says "TODO: this is only valid
for RC and UC"), there's no `AddressHandle`/`ibv_create_ah` wrapper, and `post_send` has no
per-send AH param — still true as of 0.9.2. `transport::ud` is a stub carrying this rationale in a
doc comment, not an implementation attempt.

**Server/client asymmetry**: the server doesn't know in advance what transport/mode/size/iteration
count a client wants to test — and requiring matching CLI flags on both sides is fragile and
doesn't support a long-lived server handling several different benchmark runs. So the *client*
alone specifies transport/mode/size/iterations/tx-depth via its own CLI flags, and declares them to
the server as the first thing sent over the OOB TCP connection (a `BenchmarkRequest`). The server's
CLI only needs connection-level flags (port, device) plus a `--listen` flag: without it, the server
handles exactly one client connection then exits (useful for quick one-off tests); with it, the
server loops, accepting and serving one client connection at a time, indefinitely, until killed —
so multiple benchmark runs (potentially with different transport/mode/size each time) can be driven
back-to-back against one long-lived server process.

The out-of-band handshake uses JSON via `serde_json` over a plain TCP socket, not the D3OS
`comm.rs` fixed-width byte format — a deliberate choice (confirmed with the user) to keep the Linux
side simple/debuggable now, with the understanding that D3OS's `#![no_std]` side will need its own
compatible JSON (de)serializer later to interoperate.

## Cargo.toml

```toml
[dependencies]
ibverbs = "0.9.2"
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- `ibverbs`: crates.io version dependency, not a path dep on `../rust-ibverbs`. Default features
  include `serde`, so `QueuePairEndpoint` already derives `Serialize`/`Deserialize`.
- `clap` (derive): CLI parsing, using subcommands (see below).
- `serde`/`serde_json`: needed directly because `comm.rs` defines its own `BenchmarkRequest`/
  `HandshakeAck` messages wrapping `QueuePairEndpoint`.
- No `anyhow`/`thiserror`: this is a small binary, not a library. A single alias
  `pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;` in
  `src/error.rs`, used as the return type throughout, is enough — `?` auto-converts `io::Error`,
  and the few RDMA-specific failures (WC errors) use `format!(...).into()`.

## Module layout

```
src/
  main.rs           # parse CLI, dispatch into server::run or client::run
  cli.rs            # clap derive: Cli enum { Server(ServerArgs), Client(ClientArgs) }, Transport/Mode enums
  error.rs          # Result<T> alias
  comm.rs           # OOB TCP + JSON protocol: BenchmarkRequest, HandshakeAck, sync barrier
  device.rs         # shared verbs setup: open device, create_cq, alloc_pd, allocate MR
  server.rs         # accept loop (single-shot or --listen), dispatches into bench::run per connection
  client.rs         # builds BenchmarkRequest from ClientArgs, drives one connection, dispatches into bench::run
  transport/
    mod.rs          # Transport enum + dispatch (qp_type_for(Transport), build QP w/ tx_depth)
    rc.rs           # real: builds RC QueuePairBuilder w/ set_max_send_wr/set_max_recv_wr
    uc.rs           # stub: unimplemented!("UC transport not yet implemented")
    ud.rs           # stub: unimplemented!() + doc comment citing the UD blocker (see Context)
  bench/
    mod.rs          # Mode enum + dispatch: match (transport, mode) { (Rc, Bandwidth) => ..., _ => Err(unsupported) }
    bandwidth.rs    # real: RC bandwidth windowed/pipelined loop
    latency.rs       # stub: unimplemented!("latency mode not yet implemented")
    accuracy.rs      # stub: unimplemented!("accuracy mode not yet implemented")
```

`device.rs` factors out device/context/PD setup, opened **once** per process (server opens it once
at startup and reuses the same `Context`/`ProtectionDomain` across every client connection it
serves; client opens it once for its single run). `transport::rc` only builds and returns a
connected `ibverbs::QueuePair` — it doesn't know about bandwidth/latency/accuracy; `bench::bandwidth`
owns the send/recv loop, wr_ids, and timing. `bench::run` returns a `Result` (not a panic) on an
unsupported (transport, mode) pair, so the server can reject a request cleanly instead of crashing.

## CLI (`src/cli.rs`)

Subcommands, so the server genuinely has no transport/mode/size/iterations flags to be confused
about:

```rust
#[derive(clap::Parser)]
enum Cli {
    Server(ServerArgs),
    Client(ClientArgs),
}

struct ServerArgs {
    #[arg(long, default_value_t = 18515)]
    port: u16,
    #[arg(long)]
    device: Option<String>,
    /// Keep accepting connections and serving benchmark runs one after another instead of
    /// exiting after the first.
    #[arg(long)]
    listen: bool,
}

struct ClientArgs {
    #[arg(long)]
    host: String,
    #[arg(long, default_value_t = 18515)]
    port: u16,
    #[arg(long)]
    device: Option<String>,
    #[arg(long, value_enum, default_value_t = Transport::Rc)]
    transport: Transport,                    // Rc | Uc | Ud
    #[arg(long, value_enum, default_value_t = Mode::Bandwidth)]
    mode: Mode,                              // Bandwidth | Latency | Accuracy
    #[arg(long, default_value_t = 65536)]
    size: usize,                             // message size in bytes
    #[arg(long, default_value_t = 1000)]
    iterations: usize,                       // stop condition
    #[arg(long, default_value_t = 32)]
    tx_depth: usize,                         // outstanding sends/receives allowed in flight
}
```

`Transport` and `Mode` derive both `clap::ValueEnum` and `serde::{Serialize, Deserialize}` since
they're sent over the wire in `BenchmarkRequest`. No `--duration` mode in this pass — iteration
count is the only stop condition, matching the D3OS precedent's `Config::iters`.

## OOB protocol (`src/comm.rs`)

```rust
#[derive(Serialize, Deserialize)]
struct BenchmarkRequest {
    transport: Transport,
    mode: Mode,
    msg_size: usize,
    iterations: usize,
    tx_depth: usize,
}

#[derive(Serialize, Deserialize)]
enum HandshakeAck {
    Ok { endpoint: ibverbs::QueuePairEndpoint },
    Unsupported(String),
}

#[derive(Serialize, Deserialize)]
struct ClientEndpoint {
    endpoint: ibverbs::QueuePairEndpoint,
}
```

Sequence per connection (newline-delimited JSON both ways, so a single `TcpStream` + `BufReader`
suffices — no length-prefix framing needed since one JSON object never contains a raw newline):

1. Client connects: `TcpStream::connect((host, port))`.
2. Client already knows its full config from `ClientArgs`, so it builds its local QP immediately
   (`transport::rc::build(&pd, &cq, tx_depth)` → `PreparedQueuePair`) and reads `endpoint()`.
3. Client sends `BenchmarkRequest{transport, mode, msg_size, iterations, tx_depth}`.
4. Server (already has `Context`/`ProtectionDomain` open from startup) reads the request via
   `bench::run`'s dispatch:
   - Unsupported `(transport, mode)` → server sends `HandshakeAck::Unsupported(reason)`, closes
     this connection, and (if `--listen`) loops back to accept the next one.
   - Supported → server builds its own CQ + QP (matching transport, `tx_depth`) + MR (`msg_size`),
     reads its `endpoint()`, sends `HandshakeAck::Ok { endpoint }`.
5. Client reads the ack:
   - `Unsupported(reason)` → print the error, exit (non-fatal to the server).
   - `Ok { endpoint }` → client calls `prepared.handshake(endpoint)?`, then sends
     `ClientEndpoint { endpoint: <its own endpoint> }` back to the server.
6. Server reads `ClientEndpoint`, calls `prepared.handshake(endpoint)?` — both sides now have a
   ready `QueuePair`, no further round trip needed for the QP itself.
7. Barrier (`comm::sync`, 1-byte write+read) before the timed loop, so the client doesn't start
   sending before the server has posted its initial batch of receives.
8. Barrier after the timed loop, so the server knows to stop reposting receives before either side
   tears down its QP for this connection.
9. Connection closes. If server was started with `--listen`, go back to step "accept" for the next
   client; otherwise the server process exits.

`comm.rs` exposes: `listen(port) -> Result<TcpListener>`, `accept_one(&TcpListener) ->
Result<TcpStream>`, `connect(host, port) -> Result<TcpStream>`, `send_json<T: Serialize>(&mut
TcpStream, &T) -> Result<()>`, `recv_json<T: DeserializeOwned>(&mut BufReader<&TcpStream>) ->
Result<T>`, `sync(&mut TcpStream) -> Result<()>`.

## RC bandwidth loop (`src/bench/bandwidth.rs`)

Setup, both sides: `pd.create_qp(&cq, &cq, IBV_QPT_RC).set_max_send_wr(tx_depth as
u32).set_max_recv_wr(tx_depth as u32).build()`; `cq = ctx.create_cq(2 * tx_depth as i32, 0)` (sized
comfortably above `tx_depth` so a burst of completions never overflows the poll buffer); MR is a
single `pd.allocate::<u8>(msg_size)` buffer reused for every send/receive (safe here since RC
delivery is ordered and only `tx_depth` are ever in flight against one buffer's single region —
each WR posts the same address/lkey, which is fine for a throughput measurement that doesn't need
per-message distinct payloads).

Server (receiver), windowed:
```rust
let window = tx_depth.min(iterations);
for i in 0..window {
    unsafe { qp.post_receive(&mut mr, .., i as u64)? };
}
let mut posted = window;
let mut completed = 0usize;
comm::sync(&mut stream)?;                     // "ready"
while completed < iterations {
    let wcs = cq.poll(&mut wc)?;
    for c in wcs {
        if let Some((status, vendor_err)) = c.error() {
            return Err(format!("recv WC error: {status:?} vendor_err={vendor_err}").into());
        }
        completed += 1;
        if posted < iterations {
            unsafe { qp.post_receive(&mut mr, .., posted as u64)? };
            posted += 1;
        }
    }
}
comm::sync(&mut stream)?;                     // "done draining"
println!("received {completed} messages");
```

Client (sender), windowed:
```rust
comm::sync(&mut stream)?;                     // wait for "ready"
let t0 = Instant::now();
let window = tx_depth.min(iterations);
for i in 0..window {
    unsafe { qp.post_send(&mut mr, .., i as u64)? };
}
let mut posted = window;
let mut completed = 0usize;
while completed < iterations {
    let wcs = cq.poll(&mut wc)?;
    for c in wcs {
        if let Some((status, vendor_err)) = c.error() {
            return Err(format!("send WC error: {status:?} vendor_err={vendor_err}").into());
        }
        completed += 1;
        if posted < iterations {
            unsafe { qp.post_send(&mut mr, .., posted as u64)? };
            posted += 1;
        }
    }
}
let elapsed = t0.elapsed();
comm::sync(&mut stream)?;
```

Throughput (client only, since it drives pacing):
```rust
let secs = elapsed.as_secs_f64();
let bytes = iterations as f64 * msg_size as f64;
let bw_gbps = bytes * 8.0 / secs / 1e9;
let msg_rate_mpps = iterations as f64 / secs / 1e6;
println!("{:>8}  {:>12}  {:>10}  {:>18}  {:>14}", "#bytes", "#iterations", "tx_depth", "BW avg[Gb/sec]", "MsgRate[Mpps]");
println!("{:>8}  {:>12}  {:>10}  {:>18.2}  {:>14.6}", msg_size, iterations, tx_depth, bw_gbps, msg_rate_mpps);
```

## Verification (must run on a Linux host, not this macOS session)

1. Load soft-RoCE and add a device backed by a real netdev (not `lo`):
   ```sh
   sudo modprobe rdma_rxe
   sudo rdma link add rxe0 type rxe netdev eth0   # substitute the real interface name
   ```
2. Check the device and its GID table — `endpoint()` hardcodes `gid_index = 0` with no override
   (`lib.rs:382`), so confirm what's actually at index 0:
   ```sh
   ibv_devices
   show_gids
   ```
   If index 0 is unusable for the intended topology (e.g. RoCEv1-only when RoCEv2 is needed), that's
   a real blocker to flag — not fixable from this crate version.
3. Start a long-lived server, then run one or more clients against it:
   ```sh
   # server (stays up for repeated runs)
   cargo run --release -- server --port 18515 --listen

   # client (repeatable with different transport/mode/size/iterations each time)
   cargo run --release -- client --host 127.0.0.1 --port 18515 \
       --transport rc --mode bandwidth --size 65536 --iterations 1000 --tx-depth 32
   ```
4. Expected: client prints the results table after the run; server prints the received-message
   count and, with `--listen`, goes back to waiting for the next connection rather than exiting.
5. If it fails: `dmesg | grep rxe` for kernel-side rejects; pass `--device rxe0` explicitly if more
   than one device is present; confirm nothing blocks the plain TCP handshake port locally.

## Files to create/modify

- `Cargo.toml` — add dependencies
- `src/main.rs`, `src/cli.rs`, `src/error.rs`, `src/comm.rs`, `src/device.rs`, `src/server.rs`, `src/client.rs`
- `src/transport/mod.rs`, `src/transport/rc.rs`, `src/transport/uc.rs`, `src/transport/ud.rs`
- `src/bench/mod.rs`, `src/bench/bandwidth.rs`, `src/bench/latency.rs`, `src/bench/accuracy.rs`
