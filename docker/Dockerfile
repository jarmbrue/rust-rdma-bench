# Linux build/run environment for rust-rdma-bench, usable from macOS via Docker Desktop.
#
# The point of this image is to make `ibverbs` compile: `ibverbs-sys` vendors rdma-core and
# builds it from source with CMake, then generates its FFI bindings with bindgen, so the image
# needs the rdma-core build toolchain (cmake, python3, libnl, libudev) plus libclang -- none of
# which exist on macOS.
#
# Whether a benchmark can actually *run* depends on the kernel, not on this image: it needs an
# RDMA device (real hardware, or the rdma_rxe/siw software providers). Docker Desktop's LinuxKit
# kernel has none of those modules, so on a Mac this image builds and unit-checks the code but
# `ibverbs::devices()` comes up empty. See docker/README.md.

FROM debian:bookworm-slim

# Kept in sync with ../rust-toolchain.toml, which pins the same nightly D3OS uses. Baked in at
# image build time so a `cargo build` in a fresh container doesn't stop to download a toolchain.
ARG RUST_TOOLCHAIN=nightly-2026-01-01

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        # ibverbs-sys's vendored rdma-core build
        build-essential cmake ninja-build pkg-config python3 \
        libnl-3-dev libnl-route-3-dev libudev-dev \
        # bindgen
        clang libclang-dev \
        # System rdma-core: supplies /usr/include/infiniband/verbs_api.h, which ibverbs-sys's
        # bindgen invocation picks up from the default include path (it passes -I for a
        # directory that does not exist -- the same quirk shell.nix works around on NixOS).
        # ibverbs-utils/rdma-core give you ibv_devices, ibv_devinfo and rdma(8) for poking at
        # whatever device the kernel does or does not expose.
        libibverbs-dev ibverbs-providers ibverbs-utils rdma-core iproute2 \
        curl ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | \
        sh -s -- -y --no-modify-path --profile minimal \
            --default-toolchain "$RUST_TOOLCHAIN" -c rustfmt -c clippy \
    && chmod -R a+w "$CARGO_HOME"

# Build artifacts land here rather than in the bind-mounted ./target, so a Linux build and a
# host-side build (or a second container) never fight over the same directory.
ENV CARGO_TARGET_DIR=/build/target
RUN mkdir -p /build/target && chmod -R a+w /build

WORKDIR /work
CMD ["/bin/bash"]
