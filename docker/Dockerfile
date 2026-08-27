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
# `docker buildx imagetools inspect rust:1.85-alpine`.)
FROM rust:1.85-alpine AS builder
RUN apk add --no-cache musl-dev
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
FROM alpine:3.21
RUN apk add --no-cache ca-certificates chromium
COPY --from=builder /app/webrain /usr/local/bin/webrain
# ponytail: intentionally runs as root for now — Alpine Chromium refuses to
# start as a non-root user without --no-sandbox, and launch.rs doesn't pass it
# yet. Adding a `USER webrain` here would silently break browser launches.
# Do that together with --no-sandbox handling in launch.rs (and remap the
# /root/.config + /root/.cache volumes in docker-compose.yml) when the sandbox
# story is settled.
EXPOSE 9223
ENTRYPOINT ["webrain"]
CMD ["mcp", "--http", "9223"]
