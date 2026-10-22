# ADR-001: webrain Architecture — Layered Cargo Workspace with CDP Backend + MCP Transport

- **Status:** Accepted (2026-08-01); updated 2026-10-23
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
- **Graph note (2026-10-23, re-indexed after the performance pass)**: 2589 nodes /
  6200 edges indexed (docs pages in the
  graph: JS/CSS/YAML sections; `adr_present` now true); 2 entry points
  (`webrain-cli/src/main.rs`, `.github/scripts/check_docs_orphans.py`); hotspots
  `TileEngine.new` (fan-in 111), `VectorStore.is_empty` (91), `video.Detail.as_str` (91),
  `vault.get` (81), `VectorStore.len` (64), `vault.now` (29),
  `CdpBackend.ensure_page_attached` (29), `CdpBackend.send_cmd` (27),
  `CdpBackend.eval_js` (25), `proxy.ProxyPool.parse` (23).
  Since the original record: `vault` module; `video` module + `webrain_watch` tool;
  SERP API — `webrain_serp` + `webrain serp`; MCP surface consolidated to the
  **17 intent-based tools** (guide/eval/eval_in_frame/navigate/observe/interact/extract/
  scrape/batch/crawl/search/serp/pdf/download/watch/session/vision) — legacy schemas
  + executor aliases removed (v0.6.3); session routing via `session_id` arg;
  lightpanda engine; dead-socket reconnect.
- **SERP provider tiering (2026-10-23)**: capability tiers replace engine-name
  branching — free HTML (`duckduckgo`/`bing` in parallel) → **keyed Brave API**
  (`BRAVE_API_KEY`, 20/page, no browser) → browser renders (`google`/`brave`) →
  **serpapi** (`SERPAPI_API_KEY`, `num<=100`) as the last resort. Pagination
  offsets are DATA-DRIVEN (advance by results a page actually delivered) because a
  guessed stride truncated deep requests (ddg `s=page*30` vs ~10/page → `limit=50`
  returned 11). `brave` is browser-OPTIONAL: `serp::brave_api_ready()` is the one
  predicate the core, MCP short-circuit and CLI launch branches share.
- **Concurrency & memory (2026-10-23)**: replies are dispatched by id (one reader
  task + `HashMap<u64, oneshot::Sender<Value>>`), so `send_cmd_with` holds the
  connection lock only to write and `webrain_batch` tabs no longer queue behind
  each other's 30–120 s waits. Network capture is armed per page session
  (`Option<String>`), fed by that same reader — hence no fake `eval` pump — capped
  at 2048 URLs and consumed with `mem::take`. `VectorStore` stores unit-length
  vectors (`sqrt` per query, not per tile) and vision copies each tile PNG once.
  MCP-side: exact BPE token counting is skipped above 32 KiB replies, and session
  eviction is LRU rather than an arbitrary `HashMap` key.
