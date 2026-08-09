# Containerized build environment

A Debian container with the rdma-core build toolchain and the pinned nightly, so the crate can be
compiled from macOS, where `ibverbs` cannot build at all.

```sh
docker compose build                       # once
docker compose run --rm dev cargo build    # or clippy / fmt / test
docker compose run --rm dev                # interactive shell
```

The working tree is bind-mounted at `/work`. Build artifacts go to a named volume at
`/build/target` (via `CARGO_TARGET_DIR`) rather than into the host's `./target`, so a container
build and a host build never overwrite each other's artifacts; the cargo registry — which also
holds `ibverbs-sys`'s vendored rdma-core build tree — is a named volume too, so only the first
build pays for it. `docker compose down -v` throws both away.

## What "run" means here

On a Mac, this container **builds but cannot benchmark**. Verbs are a kernel interface: the
container needs the host kernel to expose an RDMA device through `/dev/infiniband`, and Docker
Desktop's LinuxKit kernel (6.12.76-linuxkit, checked 2026-08-09) ships no InfiniBand modules at
all — not `rdma_rxe`, not `siw`, and there is no `/sys/class/infiniband`. `modprobe rdma_rxe` in
the VM fails with "module not found", so there is nothing to pass through. Both binaries start and
then fail at `device::open()`:

```
$ docker compose run --rm dev cargo run -- server
error: Function not implemented (os error 38)
```

That is the expected outcome on this machine, not a misconfiguration. `ibv_devices` (installed in
the image) says the same thing and is the quicker check.

So use the container for what it is good for — compiling, `cargo clippy`, catching type and borrow
errors before pushing — and keep running the benchmarks on ib1/ib2, which is where they are
verified anyway.

## On a host that does have RDMA devices

ib1/ib2, or a Linux VM whose kernel provides `rdma_rxe`/`siw`, need the devices passed through:

```sh
docker compose -f compose.yaml -f compose.rdma.yaml run --rm dev ibv_devices
docker compose -f compose.yaml -f compose.rdma.yaml run --rm -d server
docker compose -f compose.yaml -f compose.rdma.yaml run --rm client \
    --host server --transport uc --mode accuracy --size 4096 --iterations 10000
```

`compose.rdma.yaml` exists only to add the `/dev/infiniband` mapping, because Docker refuses to
start a container mapping a device path the host lacks. `CAP_IPC_LOCK` and an unlimited memlock
rlimit are already set in `compose.yaml` — memory registration pins pages and fails without them.

Getting a soft-RoCE device on a Mac would mean replacing Docker Desktop's VM with one running a
distro kernel that has the module (Colima/Lima on an Ubuntu image, then `modprobe rdma_rxe` and
`rdma link add rxe0 type rxe netdev eth0`). That path is untested here; `rdma_rxe` also emulates
RDMA in software, so the bandwidth and latency numbers it produces are not comparable to the
hardware runs, and a loopback pair on one host says nothing about interoperating with D3OS.
