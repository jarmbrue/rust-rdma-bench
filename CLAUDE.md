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

**None of it has been run yet.** Everything after the initial RC bandwidth commit was written on
a machine without `libibverbs`, so it has never been compiled or executed. Getting a loopback run
green is the next step, ahead of writing more benchmark code.

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

Note `bench::IDLE_TIMEOUT`: on UC a dropped message yields no completion on either side, so every
poll loop that waits on the peer is bounded by it rather than spinning forever.

## Build/run

```sh
cargo build
cargo test
```

Needs `libibverbs` on the host (same as `rust-ibverbs`), plus RDMA-capable hardware or
`rdma_rxe`/`siw` software RDMA for local testing — see
`../rust-ibverbs/ibverbs/tests/loopback.rs` for the pattern that repo uses.

Server first, then the client, which drives the run and prints the result table:

```sh
cargo run -- server --listen
cargo run -- client --host <server> --transport uc --mode accuracy --size 4096 --iterations 10000
```

The `ib1` and `ib2` git remotes are the InfiniBand-equipped test machines; work is pushed to both
plus `origin` (GitHub) and built/run there.
