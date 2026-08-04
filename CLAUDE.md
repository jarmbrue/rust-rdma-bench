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

## Status

Currently just the `cargo new` scaffold — no dependencies, no benchmark logic yet. Treat
anything below as the intended shape, not existing structure.

## Planned scope

Benchmarks across the three InfiniBand transport types:

- **RC** (Reliable Connection)
- **UC** (Unreliable Connection)
- **UD** (Unreliable Datagram)

Each transport should support three measurement modes:

- **Bandwidth**
- **Latency**
- **Accuracy** — fraction of bits/bytes correctly received (relevant for UC/UD where the
  transport doesn't guarantee delivery/integrity itself)

## Build/run

Standard Cargo project:

```sh
cargo build
cargo run
cargo test
```

Once verbs support is wired in, expect this to depend on `libibverbs` being available on the
host (same as `rust-ibverbs`) and to require RDMA-capable hardware or `rdma_rxe`/`siw`
software RDMA for local testing — check `../rust-ibverbs/ibverbs/tests/loopback.rs` for the
pattern that repo uses.
