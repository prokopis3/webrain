# ADR-001: webrain Architecture — Layered Cargo Workspace with CDP Backend + MCP Transport

- **Status:** Accepted (2026-08-01); updated 2026-08-14
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
  and dispatches 72 MCP tools.
- **webrain-cli** (bin): thin `match`-based subcommand entry point (no clap).

Key properties: `CdpBackend` is `Clone` sharing one WS; batch = tokio semaphore + one tab
per URL (parallel loads in the browser); all extraction (JSON/regex/JSON-LD/table/
autoschema/BM25) is zero-LLM by design.

## Consequences

- **Positive**: clear seams for testing; no clap dependency; single binary for CLI;
  browser state cleanly isolated behind the backend trait; concurrency via shared-WS clone.
- **Negative**: MCP layer is coupled to CDP specifics (owns the connection); HTTP session
  map adds transport complexity.
- **Graph note (2026-08-14)**: 2207 nodes / 4689 edges indexed; 1 entry point
  (`webrain-cli/src/main.rs`); hotspots `video.Detail.as_str` (fan-in 70), `vault.get` (57),
  `VectorStore.len` (52), `TileEngine.new` (46), `VectorStore.new` (32), `CdpBackend.eval_js` (25),
  `vault.now` (25), `CdpBackend.ensure_page_attached` (22).
  Since the original record: `vault` module; `video` module + `webrain_watch` tool;
  SERP API (feat/serp-api) — `webrain_serp` tool + `webrain serp` CLI subcommand;
  72 MCP tools (up from 63 — added v2 observe/interact/extract/scrape/crawl/session/vision,
  then `webrain_serp` + `webrain_eval_in_frame`; `webrain_solve_captcha` removed);
  session routing via `session_id` arg; lightpanda engine; dead-socket reconnect.
