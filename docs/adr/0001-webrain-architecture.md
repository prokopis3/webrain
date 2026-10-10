# ADR-001: webrain Architecture — Layered Cargo Workspace with CDP Backend + MCP Transport

- **Status:** Accepted (2026-08-01); updated 2026-08-27
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
  `SessionPool`, `vault` (credentials), `video` (watch + download + captions), crawl engines
  (`SpiderEngine` BFS/DFS/BestFirst, `TileEngine` PixelRAG, `VectorStore` cosine + embed
  `Endpoint`), and download/extract/batch/stealth JS.
- **webrain-mcp** (lib): stdio JSON-RPC server + HTTP transport (per-`Mcp-Session-Id`
  backend map, mint on initialize / reuse via header). `tools.rs` owns the CDP connection
  and dispatches 17 MCP tools (consolidated intent surface).
- **webrain-cli** (bin): thin `match`-based subcommand entry point (no clap).

Key properties: `CdpBackend` is `Clone` sharing one WS; batch = tokio semaphore + one tab
per URL (parallel loads in the browser); all extraction (JSON/regex/JSON-LD/table/
autoschema/BM25) is zero-LLM by design.

## Consequences

- **Positive**: clear seams for testing; no clap dependency; single binary for CLI;
  browser state cleanly isolated behind the backend trait; concurrency via shared-WS clone.
- **Negative**: MCP layer is coupled to CDP specifics (owns the connection); HTTP session
  map adds transport complexity.
- **Graph note (2026-08-27)**: 2397 nodes / 5156 edges indexed (boat-diy/ excluded); 2
  entry points (`webrain-cli/src/main.rs`, `.github/scripts/check_docs_orphans.py`); hotspots
  `video.Detail.as_str` (fan-in 75), `vault.get` (50), `VectorStore.is_empty` (34),
  `CdpBackend.ensure_page_attached` (29), `CdpBackend.send_cmd` (27), `CdpBackend.eval_js` (25),
  `VectorStore.len` (22), `video.Detail.parse` (18).
  Since the original record: `vault` module; `video` module + `webrain_watch` tool;
  SERP API — `webrain_serp` + `webrain serp`; MCP surface consolidated to the
  **17 intent-based tools** (guide/eval/eval_in_frame/navigate/observe/interact/extract/
  scrape/batch/crawl/search/serp/pdf/download/watch/session/vision) — legacy schemas
  + executor aliases removed (v0.6.3); session routing via `session_id` arg;
  lightpanda engine; dead-socket reconnect.
