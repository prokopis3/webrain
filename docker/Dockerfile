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
FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release --bin webrain

# ── Runtime stage ───────────────────────────────────────────────
FROM alpine:latest
RUN apk add --no-cache ca-certificates chromium
COPY --from=builder /app/target/release/webrain /usr/local/bin/webrain
EXPOSE 9223
ENTRYPOINT ["webrain"]
CMD ["mcp", "--http", "9223"]
