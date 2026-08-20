# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`rust-rdma-bench` is the Linux-side half of an RDMA benchmarking tool being built for a
master's thesis on the D3OS InfiniBand driver. It replaces the earlier `rust-perftest`
approach: unlike `perftest`/`rust-perftest`, it does **not** aim for wire compatibility with
Linux `perftest` — it's a purpose-built benchmark with its own protocol.

There will be two implementations that talk to each other over RDMA:

- **This repo** (`rust-rdma-bench`) — runs on Linux, built against
  [`rust-ibverbs`](../rust-ibverbs) (`../rust-ibverbs/ibverbs`) for verbs access.
- **`D3OS/os/application/rdma-bench`** (not yet created) — runs on D3OS, built against D3OS's
  own userspace RDMA library at `D3OS/os/library/rdma` (`ib_core.rs` /
  `uverbs_uapi.rs`). That library is being written to mirror `rust-ibverbs`'s API shape as
  closely as `#![no_std]` allows, so the two benchmark implementations can stay structurally
  similar and share a wire protocol/design even though they link different verbs backends.

Keep the two implementations' architecture (module layout, CLI shape, benchmark/message
types) in sync where practical — divergence here should be deliberate, not accidental. Only
this repo and `D3OS/` are edited as part of the thesis; other top-level checkouts under
`../` are read-only upstream references (see `../CLAUDE.md`).

## Scope

Benchmarks across the three InfiniBand transport types (**RC** reliable connection, **UC**
unreliable connection, **UD** unreliable datagram), each in three measurement modes:
**bandwidth**, **latency**, and **accuracy** — the fraction of bits/bytes correctly received,
which is what makes UC/UD interesting, since those transports guarantee neither delivery nor
ordering.

## Status

| | Bandwidth | Latency | Accuracy | RDMA WRITE | RDMA READ |
|---|---|---|---|---|---|
| **RC** | done | done | done | responder only | responder only |
| **UC** | done | done | done | responder only | unsupported |
| **UD** | — | — | — | — | — |

`bench::supported()` is the authority on this matrix and is checked during the handshake, so an
unimplemented combination is rejected before any RDMA resources are built.

RDMA WRITE/READ are one-sided: only the initiator's HCA produces a work completion, so `bench::rdma`
only implements `Role::Server` (registering a buffer and handing out its address/rkey via
`MemoryRegion::rkey`). This crate cannot *initiate* either op — crates.io `ibverbs` 0.9.2 has no
`rdma_write`/`rdma_read` verb and keeps `QueuePair`'s raw `ibv_qp` pointer private, so there's no way
to post one without forking that crate. D3OS's `os/library/ibverbs` is a from-scratch verbs
reimplementation (not a fork of this crate's dependency) and does have both verbs, so D3OS is the
only side that can act as client for these two modes today; this crate can only serve it.

All six RC and UC combinations have been run between `ib1` and `ib2` on real hardware (Mellanox
ConnectX-3), against the code as committed — no fixes were needed after the first hardware run.
Note that the development machine has no `libibverbs`, so changes made there cannot be compiled
locally; ib1/ib2 are where anything new gets built and exercised.

UD is blocked on the verbs binding rather than on this crate: rust-ibverbs 0.9.2 has no real UD
support (`handshake()` applies RC/UC `dest_qp_num`/`ah_attr` logic unconditionally, there is no
`ibv_create_ah` wrapper, and `post_send` takes no per-send address handle). Adding it means
patching rust-ibverbs or dropping to raw `ffi::` calls here, plus handling the 40-byte GRH that
UD prepends to every received message — which touches accuracy's header parsing and bandwidth's
byte accounting. See `src/transport/ud.rs` for the details.

The D3OS counterpart (`D3OS/os/application/rdma-bench`) now exists and is the side that moves
first: features are written there against the D3OS verbs library and ported back here. **Treat it
as the master copy** — when the two diverge, this repo is what should change, and the diff between
them should stay as small as the different backends allow. Anything Linux-only lives here alone
(clap, `--device`, `std` types); everything else — module layout, wire types, CLI surface, report
formatting — is expected to match file for file.

## Layout

- `src/cli.rs` — clap definitions; `Transport`/`Mode` enums are also the wire types. `plan()`
  resolves the optional `--mode`/`--size` lists into the matrix a client run expands to.
- `src/comm.rs` — the wire types (`BenchmarkRequest`, endpoint exchange, `AccuracyReport`) and the
  `Conn::sync()` barrier both sides use to line up before and after a run.
- `src/transport/{rc,uc,ud}.rs` — queue-pair construction per transport type.
- `src/bench/{bandwidth,latency,accuracy}.rs` — the measurement loops, each allocating its own
  memory regions since buffer count is a per-benchmark concern. They return a `Report` and print
  nothing themselves.
- `src/report.rs` — `Report`/`BandwidthStats`/`LatencyStats` and all table formatting, kept apart
  from the benchmarks so a sweep can print one header and a row per size.
- `src/{client,server}.rs` — resource setup and handshake driving for each role; the client also
  drives the suite.
- `docs/plan.md` — the design write-up the crate was built from; carries the rationale for the
  ibverbs version pin and the UD deferral in more depth than this file.

## How a run is wired up

The client picks every parameter; the server only obeys. One TCP connection carries the whole
control plane as newline-delimited JSON, in a fixed order:

1. client → `BenchmarkRequest` (transport, mode, size, iterations, tx_depth).
2. server checks `bench::supported()`, then either replies `HandshakeAck::Unsupported` and drops
   the connection, or builds its CQ/QP and replies `HandshakeAck::Ok { endpoint }`.
3. client transitions its QP with the server's endpoint, then sends `ClientEndpoint` back; the
   server transitions with that.
4. both sides call `bench::run()` with the same parameters and their own `Role`.

Both sides allocate their CQ and build their QP per connection (the server after seeing the
request, so it can size the CQ from the client's `tx_depth`); only the device context and PD are
opened once. `--listen` makes the server loop over connections; a failed run is reported and the
loop continues.

A client run is a *matrix* of (mode, size) pairs, not necessarily one benchmark: `--mode` and
`--size` both take comma-separated lists, and left out entirely they mean "all three modes" and
"every power of two from `--min-size` to `--max-size`". Each pair is an ordinary run on the wire —
its own connection, CQ and QP, the same handshake — so the server never learns that a suite is
happening; it just has to be running with `--listen`. `client::run_suite` prints one table per
mode with a row per size, skips sizes below `Mode::min_msg_size()`, and reports a failing run in
place rather than aborting the sweep.

Inside a benchmark, `Conn::sync()` is the only thing keeping the two sides in step (e.g. receives
must be posted before the sender starts). The calls are positional and must stay paired one-to-one
across the client and server halves — an unmatched `sync()` deadlocks the run rather than failing.
The same connection then carries results back where the receiving side computed them (accuracy
sends its `AccuracyReport` to the client, which prints it). Each `sync()` takes a label naming the
barrier; setting `RDMA_BENCH_DEBUG=1` makes both sides print every barrier they reach, pass or fail
(with how long the wait took), which is how a mispaired-barrier hang gets diagnosed. The label is
local — it never goes over the wire — so it costs nothing but must still be kept recognisably
paired with the peer's. Leave the variable unset for real measurements: the tracing writes to
stderr from inside the timed section.

Note `bench::IDLE_TIMEOUT`: on UC a dropped message yields no completion on either side, so every
poll loop that waits on the peer is bounded by it rather than spinning forever. Any new
peer-waiting loop needs the same backstop.

## Build/run

```sh
cargo build
cargo clippy
cargo fmt
```

There is no test suite — `cargo test` compiles but runs nothing, and nothing here is verified
except by running the two binaries against each other on ib1/ib2. Treat "it builds" as the weakest
possible signal and say so when reporting.

From the macOS development machine, `docker compose run --rm dev cargo build` compiles the crate in
the Linux container defined by `docker/Dockerfile` — the only way to type-check a change here,
since macOS has no verbs. It cannot benchmark: Docker Desktop's kernel exposes no RDMA device. See
`docker/README.md`.

The toolchain is pinned in `rust-toolchain.toml` to the same nightly D3OS uses, so this crate and
its D3OS-side twin are built with the same compiler; bump the two together, not one alone.

The `ibverbs` dependency comes from crates.io, deliberately *not* as a path dependency on
`../rust-ibverbs` — that checkout has local modifications and is reference material only. It still
is the place to read when a verbs API question comes up, since it's the same source. Building it
needs the rdma-core build toolchain (`ibverbs-sys` vendors rdma-core and builds it via CMake, plus
bindgen/libclang); `shell.nix` sets all of that up, including the include-path workarounds bindgen
needs on NixOS, so build inside `nix-shell` where a Nix environment is in play. Actually running
also needs RDMA-capable hardware or `rdma_rxe`/`siw` software RDMA — see
`../rust-ibverbs/ibverbs/tests/loopback.rs` for the pattern that repo uses.

Server first, then the client, which drives the run and prints the result table:

```sh
cargo run -- server --listen
# one benchmark
cargo run -- client --host <server> --transport uc --mode accuracy --size 4096 --iterations 10000
# the whole suite: every mode, every power of two from 8 B to 64 KiB
cargo run -- client --host <server> --transport uc
```

The `ib1` and `ib2` git remotes are the InfiniBand-equipped test machines; work is pushed to both
plus `origin` (GitHub) and built/run there.

## Comment style

This is thesis code, so the comments carry the reasoning: module- and item-level doc comments
explain what a piece measures and why it is shaped that way (see `bench/accuracy.rs`'s header, or
the doc comment on `IDLE_TIMEOUT`), and inline comments justify non-obvious choices rather than
restating the code. Match that when adding to it — a new benchmark mode without its "why" reads as
out of place here.
