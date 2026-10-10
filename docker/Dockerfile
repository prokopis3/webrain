# syntax=docker/dockerfile:1
# webrain — self-contained MCP browser-automation server.
#
# Build from the repo root (the build context must be the repo root so the
# workspace crates are visible):
#
#   docker build -f docker/Dockerfile -t webrain .
#   docker run -p 9223:9223 webrain mcp --http 9223
#   docker compose -f docker/docker-compose.yml up -d
#
# Multi-arch:
#   docker buildx build --platform=linux/amd64,linux/arm64 -t ghcr.io/prokopis3/webrain -f docker/Dockerfile .

# ── Build stage ─────────────────────────────────────────────────
# Pinned to a concrete minor version (not a floating `latest`) so rebuilds are
# reproducible. (A full @sha256: digest pin is the next step — obtain it with
# `docker buildx imagetools inspect rust:1.88-slim-bookworm`.)
#
# Debian, not Alpine; 1.88, not 1.85. Two independent reasons:
#   * this toolchain must MATCH THE RUNTIME. The builder's libc decides the
#     binary's libc, and the runtime has to be glibc so it can exec Obscura
#     (see the runtime stage). A musl-built binary in a glibc image is a
#     mismatch waiting to surface at the first browser launch.
#   * the workspace resolves deps needing rustc 1.88, so the 1.85 toolchain
#     fails `cargo build --release` outright.
# `musl-dev` is gone with Alpine: it was there to link against musl, which is
# exactly what we no longer want.
FROM rust:1.88-slim-bookworm AS builder
# BuildKit's automatic platform args exist only in the global scope BEFORE the
# first FROM, so `$TARGETARCH` used inside a stage without redeclaring it here
# expands to EMPTY. That silently collapsed the target cache below into one
# shared key for every architecture, which is the opposite of what its comment
# claims. Docker's own lint flagged it (UndefinedVar, line 49).
ARG TARGETARCH
# Link-time build deps. The slim Rust image normally already carries these;
# installing them explicitly means a future base change cannot remove one
# silently and surface as an unexplained link error.
RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
# Copy only the workspace manifests + crate sources — the previous `COPY . .`
# baked the entire repo into the image layers and invalidated this layer on any
# repo change, forcing a full recompile. (Secrets are already excluded via
# .dockerignore's .env*/key patterns.)
COPY Cargo.toml Cargo.lock ./
COPY webrain-core webrain-core
COPY webrain-mcp webrain-mcp
COPY webrain-cli webrain-cli
# Reuse the cargo registry + target caches across builds (BuildKit) so a source
# change doesn't recompile the whole dependency tree. The target cache is keyed
# by TARGETARCH so amd64/arm64 artifacts never mix, and the built binary is
# copied OUT of the ephemeral cache mount within the same RUN (a cache mount's
# contents are discarded when the RUN ends and never reach the image layer).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target,id=webrain-target-$TARGETARCH \
    cargo build --release --bin webrain \
    && cp /app/target/release/webrain /app/webrain

# ── Runtime stage ───────────────────────────────────────────────
# Debian, not Alpine, and for a MEASURED reason: this image has to EXEC the
# browser engines, and Obscura's Linux release is `x86_64-unknown-linux-gnu`,
# which needs glibc >= 2.35. A musl base cannot exec it at all, so the failure
# would land per browser session, at exec, and never at build.
#   $ docker run --rm alpine:3.21    sh -c 'ldd --version'  -> musl libc (x86_64)
#   $ docker run --rm debian:12-slim sh -c 'ldd --version'  -> glibc 2.36
FROM debian:12-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates chromium \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/webrain /usr/local/bin/webrain
# ponytail: intentionally runs as root for now — Chromium refuses to start as a
# non-root user without --no-sandbox, and launch.rs doesn't pass it yet. Adding
# a `USER webrain` here would silently break browser launches.
# Do that together with --no-sandbox handling in launch.rs (and remap the
# /root/.config + /root/.cache volumes in docker-compose.yml) when the sandbox
# story is settled.
EXPOSE 9223
ENTRYPOINT ["webrain"]
CMD ["mcp", "--http", "9223"]
