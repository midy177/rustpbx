# syntax=docker/dockerfile:1.7
#
# Multi-stage image for the three distributed RustPBX components:
#   control — Control Plane (gRPC + HTTP admin console; SPA embedded via rust-embed)
#   edge    — SIP Edge Gateway (signaling; links the full `rustpbx` media crate)
#   worker  — Media Worker (B2BUA + RTP; links the full `rustpbx` media crate)
#
# Build one component (from the repo root):
#   docker build -f deploy/Dockerfile --target control -t rustpbx-control .
#   docker build -f deploy/Dockerfile --target edge    -t rustpbx-edge .
#   docker build -f deploy/Dockerfile --target worker  -t rustpbx-worker .
#
# Multi-arch:
#   docker buildx build --platform linux/amd64,linux/arm64 \
#       -f deploy/Dockerfile --target worker -t rustpbx-worker .
#
# `control` is the only target that needs no media libraries at runtime, so it
# produces the smallest image. `edge`/`worker` link the `rustpbx` crate and
# therefore need libopus0 at runtime.

# ─── web: build the control-plane admin SPA ──────────────────────────────────
# Built separately so the Rust stage doesn't need bun on PATH.
FROM oven/bun:1 AS web
WORKDIR /web
COPY crates/rustpbx-control/web .
RUN bun install && bun run build

# ─── builder: compile all three binaries ─────────────────────────────────────
FROM rust:1-bookworm AS builder
ARG DEBIAN_FRONTEND=noninteractive
# Build-time deps for the rustpbx crate (audio-codec/openssl/alsa). Runtime
# copies are installed in the per-target stages below.
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake \
        pkg-config \
        libssl-dev \
        libopus-dev \
        libasound2-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
# Copy the workspace (.dockerignore drops target/, node_modules, web/dist).
COPY . .
# Pre-built SPA from the web stage; the control build.rs honors this env var
# and skips re-running bun, embedding the SPA via rust-embed.
COPY --from=web /web/dist crates/rustpbx-control/web/dist
ENV RUSTPBX_SKIP_WEB_BUILD=1

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release \
        -p rustpbx-control -p rustpbx-edge -p rustpbx-worker && \
    cp /app/target/release/rustpbx-control \
       /app/target/release/rustpbx-edge \
       /app/target/release/rustpbx-worker /tmp/

# ─── control runtime (lightweight — no media libs) ───────────────────────────
FROM debian:bookworm-slim AS control
LABEL org.opencontainers.image.source="https://github.com/restsend/rustpbx"
LABEL org.opencontainers.image.description="RustPBX Control Plane"
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*
ENV LANG=C.UTF-8 TZ=UTC RUST_LOG=info
WORKDIR /app
COPY --from=builder /tmp/rustpbx-control /app/rustpbx-control
# HTTP admin console (9080) + gRPC (9090). Raft transport (if clustered) is on
# its own port from [raft].addr — expose via the config.
EXPOSE 9080 9090
ENTRYPOINT ["/app/rustpbx-control"]

# ─── edge runtime (signaling gateway) ────────────────────────────────────────
FROM debian:bookworm-slim AS edge
LABEL org.opencontainers.image.source="https://github.com/restsend/rustpbx"
LABEL org.opencontainers.image.description="RustPBX SIP Edge Gateway"
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates tzdata libopus0 \
    && rm -rf /var/lib/apt/lists/*
ENV LANG=C.UTF-8 TZ=UTC RUST_LOG=info
WORKDIR /app
COPY --from=builder /tmp/rustpbx-edge /app/rustpbx-edge
# SIP signaling. Edge↔Worker is TCP; carrier side may be UDP/TCP/TLS/WS/WSS.
# health_addr (default 8081) is set in the config.
EXPOSE 5060/udp 5060/tcp
ENTRYPOINT ["/app/rustpbx-edge"]

# ─── worker runtime (media) ──────────────────────────────────────────────────
FROM debian:bookworm-slim AS worker
LABEL org.opencontainers.image.source="https://github.com/restsend/rustpbx"
LABEL org.opencontainers.image.description="RustPBX Media Worker"
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates tzdata libopus0 \
    && rm -rf /var/lib/apt/lists/*
ENV LANG=C.UTF-8 TZ=UTC RUST_LOG=info
WORKDIR /app
COPY --from=builder /tmp/rustpbx-worker /app/rustpbx-worker
# Internal SIP (5060/5070) + the RTP media range. With hostNetwork the RTP
# ports are reachable directly on the node IP.
EXPOSE 5060/udp 5060/tcp 5070/udp 5070/tcp 20000-40000/udp
ENTRYPOINT ["/app/rustpbx-worker"]
