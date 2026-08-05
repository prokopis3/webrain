# ADR-001: webrain Architecture — Layered Cargo Workspace with CDP Backend + MCP Transport

- **Status:** Accepted (2026-08-01); updated 2026-08-05
- **Project:** webrain (`d:\Windows\Documents\Programming\Projects\Rust\webrain`)
- **Related docs:** `docs/ARCHITECTURE.md`, `ARCHITECTURE.md` (historical)

## Context

webrain is a generic LLM browser automation & web scraping agent written in Rust.
It needs to drive a real browser (via Chrome DevTools Protocol over WebSocket), expose
that capability over both stdio and HTTP JSON-RPC (MCP), and offer crawl/extract/vision
tooling — without any LLM dependency at runtime (all extraction is zero-LLM).

## Decision

Adopt a three-crate Cargo workspace with strict layering:

- **webrain-core** (lib, no binary): the engine. Owns `BrowserBackend` trait,
  `CdpBackend` (single shared WebSocket, `Clone`, per-tab `*_session` ops),
  `SessionPool`, crawl engines (`SpiderEngine` BFS/DFS/BestFirst, `TileEngine` PixelRAG,
  `VectorStore` cosine + embed `Endpoint`), and download/extract/batch/stealth JS.
- **webrain-mcp** (lib): stdio JSON-RPC server + HTTP transport (per-`Mcp-Session-Id`
  backend map, mint on initialize / reuse via header). `tools.rs` owns the CDP connection
  and dispatches 51 MCP tools.
- **webrain-cli** (bin): thin `match`-based subcommand entry point (no clap).

Key properties: `CdpBackend` is `Clone` sharing one WS; batch = tokio semaphore + one tab
per URL (parallel loads in the browser); all extraction (JSON/regex/JSON-LD/table/
autoschema/BM25) is zero-LLM by design.

## Consequences

- **Positive**: clear seams for testing; no clap dependency; single binary for CLI;
  browser state cleanly isolated behind the backend trait; concurrency via shared-WS clone.
- **Negative**: MCP layer is coupled to CDP specifics (owns the connection); HTTP session
  map adds transport complexity.
- **Graph note (2026-08-05)**: 875 nodes / 2287 edges indexed; 3 entry points
  (`webrain-cli/src/main.rs`, `scripts/stealth_solve.py`, `skills/webrain/scripts/stealth_solve.py`);
  hotspots `vault.get` (fan-in 26), `CdpBackend.ensure_page_attached` (13),
  `CdpBackend.send_cmd_with` (13), `BrowserBackend.evaluate` (11), `VectorStore.len` (11).
  Since the original record: `vault` module added (top hotspot); 51 MCP tools (up from 34 —
  added sitemap/page_info/save|restore_state/pdf_*/open|close_session/login/cookies/clean/vision_retrieve);
  session routing via `session_id` arg; lightpanda engine; dead-socket reconnect (0.3.0/0.3.1).
