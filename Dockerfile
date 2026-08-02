# syntax=docker/dockerfile:1
# webrain — self-contained MCP browser-automation server.
#
#   docker build -t webrain .
#   docker run -p 9223:9223 webrain mcp --http 9223
#
# Multi-arch: docker buildx build --platform=linux/amd64,linux/arm64 -t ghcr.io/your-org/webrain .

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
