# ADR-001: webrain Architecture — Layered Cargo Workspace with CDP Backend + MCP Transport

- **Status:** Accepted (2026-08-01); updated 2026-08-06
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
  and dispatches 15 consolidated intent-based MCP tools (`map_surface()` routes each
  consolidated `what`/`action`/`op`/`mode` selector to the legacy executor; old names
  still dispatch — backward compatible).
- **webrain-cli** (bin): thin `match`-based subcommand entry point (no clap).

Key properties: `CdpBackend` is `Clone` sharing one WS; batch = tokio semaphore + one tab
per URL (parallel loads in the browser); all extraction (JSON/regex/JSON-LD/table/
autoschema/BM25) is zero-LLM by design.

## Consequences

- **Positive**: clear seams for testing; no clap dependency; single binary for CLI;
  browser state cleanly isolated behind the backend trait; concurrency via shared-WS clone.
- **Negative**: MCP layer is coupled to CDP specifics (owns the connection); HTTP session
  map adds transport complexity.
- **Graph note (2026-08-06)**: 1188 nodes / 3217 edges indexed; 3 entry points
  (`webrain-cli/src/main.rs`, `scripts/stealth_solve.py`, `skills/webrain/scripts/stealth_solve.py`);
  hotspots `video.Detail.as_str` (fan-in 58), `vault.get` (41), `CdpBackend.eval_js` (23),
  `CdpBackend.ensure_page_attached` (20), `install.browsers_dir` (16), `CdpBackend.send_cmd` (16),
  `VectorStore.len` (15), `CdpBackend.send_cmd_with` (15), `BrowserBackend.evaluate` (12).
  Since the original record: `vault` module; `video` module + `webrain_watch` tool (now top
  hotspot); 63 MCP tools accumulated, then compressed to a 15-tool consolidated surface
  (navigate/observe/interact/extract/scrape/batch/crawl/search/pdf/download/watch/session/
  vision/eval/guide) with per-capability selectors routed via `map_surface()`;
  session routing via `session_id` arg; lightpanda engine; dead-socket reconnect.
  PixelRAG (`TileEngine`/`vision`) migrated its vision path from the old
  `Qwen3-VL-Embedding-2B` @ vLLM:8000 fallback to the bundled local vision model
  (`Qwen3-VL-2B` via llama-server, `webrain install vision`): embeddings still
  power cosine retrieval, `vision::describe_tiles` captions captured tiles for
  real understanding.
