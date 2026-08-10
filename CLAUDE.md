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

| | Bandwidth | Latency | Accuracy |
|---|---|---|---|
| **RC** | done | done | done |
| **UC** | done | done | done |
| **UD** | — | — | — |

`bench::supported()` is the authority on this matrix and is checked during the handshake, so an
unimplemented combination is rejected before any RDMA resources are built.

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

The D3OS counterpart (`D3OS/os/application/rdma-bench`) does not exist yet.

## Layout

- `src/cli.rs` — clap definitions; `Transport`/`Mode` enums are also the wire types.
- `src/comm.rs` — out-of-band TCP handshake (`BenchmarkRequest`, endpoint exchange) and the
  `Conn::sync()` barrier both sides use to line up before and after a run.
- `src/transport/{rc,uc,ud}.rs` — queue-pair construction per transport type.
- `src/bench/{bandwidth,latency,accuracy}.rs` — the measurement loops, each allocating its own
  memory regions since buffer count is a per-benchmark concern.
- `src/{client,server}.rs` — resource setup and handshake driving for each role.
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

Note the asymmetry: the server allocates its CQ per connection (sized from the client's
`tx_depth`) while the client allocates once up front, and the server builds its QP *after* seeing
the request. `--listen` makes the server loop over connections; a failed run is reported and the
loop continues.

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
cargo run -- client --host <server> --transport uc --mode accuracy --size 4096 --iterations 10000
```

The `ib1` and `ib2` git remotes are the InfiniBand-equipped test machines; work is pushed to both
plus `origin` (GitHub) and built/run there.

## Comment style

This is thesis code, so the comments carry the reasoning: module- and item-level doc comments
explain what a piece measures and why it is shaped that way (see `bench/accuracy.rs`'s header, or
the doc comment on `IDLE_TIMEOUT`), and inline comments justify non-obvious choices rather than
restating the code. Match that when adding to it — a new benchmark mode without its "why" reads as
out of place here.
