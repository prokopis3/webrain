# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **core**: adaptive selectors (Scrapling-style `adaptive=True`) — `webrain_extract_json`
  gains `adaptive: bool`. When the base selector matches 0 items (site redesigned / class
  renamed), the extractor auto-relocates to elements that still contain ≥2 of the field
  selectors, keeping only the deepest (row-level) candidates. Zero-LLM structural
  re-anchoring, all in-page via one `evaluate()`.
- **core**: 3500-domain tracker blocklist — `webrain_navigate`/`webrain_batch` gain
  `block_trackers: bool`. Ported from anudeepND/blacklist via
  `scripts/port_blocklist.ps1` into `webrain-core/data/tracker_domains.txt`, embedded at
  compile time (`include_str!`), lazy-parsed once. Applied to CDP only when opted in
  (~35KB over CDP per navigate) — the default fast path stays at the 28 wildcards.
- **core**: batch consolidation — 4 near-identical batch fns (fetch/extract/interact/
  screenshot, ~685–892 lines) collapsed into one generic `batch_map<F, Fut>` helper + 4
  thin wrappers (-74 net lines). One tab lifecycle (open → session → navigate → op →
  close) shared by every op, so a fix covers all callers.
- **api**: `webrain_batch` gains `per_backend_concurrency` — bounds tabs per CDP backend
  when `cdp_urls` is set (memory cap: total tabs = this × backends; default = concurrency).
- **perf**: `bm25_filter` now precomputes per-term doc-frequency once
  (O(docs·terms)) instead of re-scanning all docs per (doc, term) inside the
  score loop (O(docs²·terms)). Kills the flagged linear-scan-in-loop hot path.
- **perf**: hand-rolled `base64_encode` (24 ln, per-chunk allocations) replaced
  with `base64::engine::general_purpose::STANDARD.encode` — SIMD-accelerated
  stdlib, already a dep. Real speedup on the `webrain_pixel` tile path.
- **docs**: `docs/adr/0001-webrain-architecture.md` — Architecture Decision Record
  for the layered Cargo workspace (webrain-core engine + CDP backend, webrain-mcp
  transport with 34 tools, webrain-cli thin binary).
- **mcp**: session management tools — `webrain_open_session`, `webrain_close_session`,
  `webrain_list_sessions`. The LLM can now create named browser session pools
  (each with optional `cdp_url` for per-session browser routing), list active
  pools, and destroy them. This is the architectural unlock for auto-subagent
  orchestration: the LLM opens N sessions across different CDP_URLs, then farms
  MCP requests with different `Mcp-Session-Id` headers across parallel subagents.
  The existing `Mcp-Session-Id` routing + `HttpState` map already had the
  infrastructure — ~50 lines of MCP tool wrappers were added.
  `CdpBackend::connect_with_url()` added for per-session CDP routing.
- **pdf**: `webrain_pdf_extract` now uses the **Firecrawl `pdf-inspector`** engine
  (pure Rust, built on lopdf) instead of hand-rolled `extract_text_chunks`.
  Proper ToUnicode CMap decoding fixes the LaTeX/CID-font bug that previously
  failed all 9 pages of the Docling paper. New output: full `markdown`
  (headings/lists/tables/bold-italic), `pdf_type` (TextBased/Scanned/Mixed),
  `confidence`, `has_encoding_issues`, and `layout` (`is_complex`,
  `pages_with_tables`, `pages_with_columns`) + per-page `texts` with
  `needs_ocr`. Same engine with or without `--features pdfium`. Verified on 2
  arXiv papers: Docling 9p/41.9K markdown chars (tables on pages 3,5), RAG
  Survey 21p/111.9K markdown chars (tables on 1,6,13,14), 0 encoding errors.
- **pdf**: `webrain_pdf_extract` batch mode is now **concurrent** — PDF parsing
  is CPU-bound, so `pdf_extract_batch` runs a fixed worker pool (stdlib
  `thread::scope`, capped at `available_parallelism()`) and the MCP handler
  calls it via `spawn_blocking` so a huge batch never stalls a tokio worker.
  Verified: 5 arXiv PDFs / 142 pages / 561,896 markdown chars in ~14.4s with
  tables detected across every file.
- **pdf**: `webrain_pdf_images` is now **zero-dependency** — uses lopdf + `image`
  crate + `flate2` (all pure Rust) to extract embedded images as base64 PNGs.
  Handles DCTDecode (JPEG) and FlateDecode (zlib-compressed raw pixels, 8bpp
  DeviceRGB/DeviceGray). Works in the default build, no `--features pdfium`
  needed. Skips JPEG2000/CCITT/JBIG2 — use `webrain_pdf_render` (pdfium) for
  those. Also integrated into `webrain_pdf_extract` output as `images[]`.
  Previously required `--features pdfium` + `pdfium.dll` (~80MB system dep).
- **pdf**: `webrain_pdf_images` (feature `pdfium`) — extract embedded
  images/figures from PDF pages as base64 PNGs (Docling `generate_picture_images`
  / MarkItDown `page.images` pattern). **Now superseded by the zero-dep path;
  the pdfium feature only adds JPEG2000/CCITT fallback + `pdf_render`.**
- **pdf**: `webrain_pdf_render` (feature `pdfium`) — render PDF pages as base64
  PNGs for vision-model reading. Optional `tile_size` (e.g. 800) splits each
  page into square tiles for efficient multi-page vision processing
  (PixelRAG-style). Bypasses font-encoding issues entirely.
- **pdf**: `webrain_download` now works without a browser backend (same
  no-browser bypass as `webrain_fetch_http`). Uses `ureq` internally.
- **clean**: `webrain_clean` — in-page JS text cleaning (strip nav/footer/social,
  word threshold). Zero-LLM, zero deps.
- **cache**: SHA-256 disk cache (`cache_read`/`cache_write`) for crawl results.
  Direct token-cost win — re-crawling same URL costs same prompt tokens.
- **mcp**: `with_token_cost()` now uses **real BPE tokenization** via `tiktoken-rs`
  (cl100k_base vocab compiled in with `include_bytes!` — zero runtime download,
  zero infrastructure, matches OpenAI-style billing) instead of the chars/4
  heuristic. Every tool response carries `tokens: {chars, est_tokens}` computed
  at the serialization choke point (stdio + HTTP). Lazy `OnceLock` builds the
  tokenizer once and reuses it.
- **batch**: `webrain_batch` gains `op=interact` — runs an async JS interaction
  (click "Load More" loop, infinite-scroll, form fill) in PARALLEL tabs (one per
  URL, semaphore-bounded), then optionally extracts a CSS schema. One call
  replaces N serial agent loops for N independent interactive sites. Verified
  live: button-click → 132 products, infinite-scrolling → products in a single
  parallel call. Also: optional `output` path persists the full batch payload to
  disk (survives temp-file GC between turns).
- **batch**: `webrain_batch` gains optional `cdp_urls` — round-robins URLs across
  N CDP backends (each browser = own proxy/cookies/fingerprint). The per-proxy
  isolation game-changer: one call fans out across N exit IPs, no subagents
  needed. All ops (fetch/extract/interact/screenshot) share one per-backend
  runner, so a fix covers every caller. `batch_screenshot` now honors `NavOpts`
  (network_idle/disable_resources/wait_selector). Benchmarked (4 URLs): single
  backend 2.0s warm, multi-backend (2 Chrome) 2.3s — same throughput expected
  (single-backend already parallelizes tabs); multi-backend's win is isolation,
  not raw speed.
- **navigate/batch**: request-quality params (Scrapling-style) — `disable_resources`
  (block font/image/media/stylesheet), `network_idle`, `wait_selector` +
  `wait_selector_state` (attached|visible|hidden|detached), `css_selector`
  narrowing. Threaded through ONE shared root (`navigate_opts` +
  `navigate_session_opts`), exposed on `webrain_navigate` and `webrain_batch`.
- **deploy**: multi-stage `Dockerfile` (rust:alpine builder → alpine + chromium
  runtime). `docker build -t webrain . && docker run -p 9223:9223 webrain`.
- **cli**: `--doctor` diagnostics — probes MCP port (9223), CDP ports (9222,
  9224), browser name/version, Python/stealth deps, cargo version. Exit 0 when
  healthy, 2 when a browser or MCP is missing. Zero-config health check.
- **mcp**: `webrain_fetch_http` now works WITHOUT a browser backend
  (special-cased in `handle_rpc` alongside `webrain_guide`). Pure `ureq` HTTP
  GET → `{status, url, text}`. 10-100× faster for static pages, zero memory.
- **mcp**: AGENT_GUIDE now documents tab management (`webrain_tab`) and the
  parallel/multi-browser subagent pattern (per-session CDP isolation).
- **skill**: self-contained `skills/webrain/` marketplace skill (claude-video
  `watch` pattern) — SKILL.md contract + bundled `scripts/`: `preflight.py`
  (MCP/CDP status, `--check` silent exit), `stealth_solve.py` (real-Chrome
  Cloudflare/CAPTCHA bypass + login + cookie export, self-contained copy),
  `build-skill.sh` (`dist/webrain.skill`). Installable across hosts via
  `npx skills add <repo> -g`; `scripts/stealth_solve.py` restored at repo root
  so the guide/AGENTS.md references resolve.
- **mcp**: `webrain_guide` tool — the agent decision guide (browser selection,
  challenge bypass via `scripts/stealth_solve.py`, extraction matrix) embedded
  in the binary, so ANY LLM connected over MCP can fetch it via `tools/list`
  without repo files. `webrain_navigate` description now documents the
  `challenge` field contract.
- **guide**: `docs/AGENT_DECISION_GUIDE.md` — agent-facing decision guide for
  the MCP tools: browser selection (real Chrome vs obscura vs lightpanda vs
  `fetch_http`), the challenge/anti-bot decision tree (via the `challenge`
  field + `scripts/stealth_solve.py` chrome-way), extraction tool matrix, and
  the from-scratch discovery workflow. Wired into
  `.github/copilot-instructions.md` and root `AGENTS.md` (cross-agent).
- **antibot**: challenge/block detector — `PageState` gains `challenge`
  (`cloudflare_challenge` | `blocked` | `captcha`) and `webrain_navigate` /
  `webrain_snapshot` / `webrain_search` surface it, so a CF challenge or
  forbidden page is flagged instead of returned as an empty page (crawl4ai
  `antibot_detector` pattern; title+visible-text markers, no HTML/network).
  Verified: `nowsecure.nl` → `cloudflare_challenge`; normal pages → null.
- **stealth**: tracker blocklist — `Network.setBlockedURLs` (28 patterns:
  google-analytics, googletagmanager, doubleclick, facebook.net, hotjar,
  newrelic, mixpanel, segment, amplitude, fullstory, mouseflow, criteo,
  taboola, outbrain, …) applied once in the shared `attach_and_init`, so
  every tab blocks analytics/ad/fingerprinting hosts before they load
  (obscura / camofox-browser pattern). CDP-native, zero JS, no
  page-function impact.
- **stealth**: canvas + audio fingerprint noise in STEALTH_JS —
  deterministic per-context seed perturbs `getImageData` pixels and
  `getChannelData` samples, so the canvas/audio hash differs across
  contexts but is stable within one (camoufox seed pattern, JS-level
  approximation). Verified live: prototype methods are wrapped, page loads
  normally.
- **extraction**: `webrain_extract_json` full crawl4ai schema — `base_fields`
  (attributes from the container), field types `regex`/`nested`/`nested_list`/
  `list`, and `source` (sibling-element targeting via `+ tr`). All in-page JS
  via `evaluate()`, zero deps, zero serialisation overhead.
- **regex**: 15 new built-in patterns (`currency`, `percentage`, `number`,
  `date_iso`, `date_us`, `time24h`, `ipv6`, `hex_color`, `postal_us`,
  `postal_uk`, `credit_card`, `iban`, `mac_addr`, `twitter_handle`,
  `hashtag`). Total now 23 patterns, matching crawl4ai's full
  `RegexExtractionStrategy` surface.
- **core**: `SpiderEngine` DFS + domain filter + URL seeding.
  `CrawlStrategy::{Bfs,Dfs}` (pop-front for BFS, pop-back for DFS),
  `with_same_domain` / `with_allowed_domains` builder, 400ms post-navigate
  settle for JS-rendered nav links.
- **core**: link-only prefetch — new `BrowserBackend::discover_links` fast
  path (`no_content` spider mode) skips innerText + full-load fallback
  (crawl4ai `prefetch=True`). 100-page DFS crawl in ~8s.
- **core**: `respect_robots` spider mode — fetches `robots.txt` once for the
  seed origin, honors `Disallow:` prefixes (crawl4ai `check_robots_txt`).
- **mcp**: `webrain_semantic_tree` — AX-tree text snapshot for the LLM
  (lightpanda `LP.getSemanticTree` style), plus raw JSON.
- **mcp**: HTTP transport — `webrain mcp --http <port>` serves MCP over POST
  with **`Mcp-Session-Id` session persistence** (lightpanda `mcp --port` style):
  one CdpBackend per session, so a client's navigate→extract sequence survives
  separate HTTP requests. Required for VS Code/Copilot HTTP MCP clients.
- **filter**: `webrain_bm25` (browsemind BM25 filter / crawl4ai
  `ContentRelevanceFilter`) — rank text items by query relevance, keep top_k.
  Zero LLM, stdlib-only BM25 (k1=1.5, b=0.75).
- **core**: concurrent multi-tab batch — `webrain_batch` gains `concurrency`
  (default 4). `CdpBackend` is now `Clone` with per-session ops
  (`navigate_session`/`eval_session`/`screenshot_session` via
  `send_cmd_with(session)`), so each URL drives its OWN tab and pages load in
  PARALLEL in the browser (crawl4ai `arun_many` + `MemoryAdaptiveDispatcher`).
  A tokio semaphore bounds in-flight tabs. 16-page ecommerce batch: 14.1s @ 5.
- **validation**: `webrain_validate_urls` (browsemind `seed(from_links,
  validate=True)`) — HEAD-then-GET probe, marks alive vs dead (404/5xx/errors).
- **extraction**: `webrain_get_jsonld` (browsemind `extract_identity`) — parse
  `<script type=application/ld+json>` schema.org blocks, zero LLM.
- **extraction**: `webrain_table` (browsemind `extract_table`) — HTML tables to
  JSON row objects, zero LLM.
- **extraction**: `webrain_autoschema` — detect repeated container patterns,
  returns candidate base-selectors for the LLM to build a schema (browsemind
  auto-detect CSS schema, zero LLM).
- **fetch**: `webrain_fetch_http` (browsemind `http_crawl`) — no-browser ureq
  GET, 10-100x faster than navigation, zero memory; static pages only.
- **interaction**: `webrain_scan` (browsemind `scan_full_page`) — auto-scroll
  to trigger infinite-scroll / load-more before extraction.
- **core**: `CrawlStrategy::BestFirst` (crawl4ai `BestFirstCrawlingStrategy`) —
  keyword-relevance scored frontier (URL substring hits), insertion-sorted.
  `webrain_spider` + `webrain spider` take `keywords`/`--keywords`.
- **core**: STEALTH_JS upgraded — self-destructing IIFE (no `window.setXxx`
  survivors), more surfaces: `hardwareConcurrency`, `deviceMemory`,
  `platform`, `oscpu`, `vendor`, WebGL vendor/renderer hook
  (camoufox addInitScript pattern).
- **cli**: `webrain spider` gains `--dfs`, `--depth N`, `--pages N`,
  `--no-same-domain`, `--discover-only`, `--respect-robots`, `--json-urls`.
- **core**: CDP network capture — `Network.requestWillBeSent` URLs are buffered
  while a capture window is open and exposed via `capture_media(url, wait_ms)`.
  Catches JS-loaded media/player-API requests (e.g. the antenna Phaistos player's
  `PlayerDataGraphQL_v2`) that are invisible to the page's resource-timing buffer.
- **tools**: `webrain_media` — discover media URLs with two tiers (browsemind
  `find_media_urls` pattern): with a `url` it captures the full network load via
  CDP (reliable); without, it scans the Performance API plus
  `<video>`/`<audio>`/`<source>` elements. Optional `wait_ms` lets the player fire.
- **tools**: `webrain_extract_regex` — in-page zero-LLM regex extraction with 8
  built-in patterns (`email`/`url`/`phone`/`price`/`date`/`time`/`ip`/`uuid`)
  and custom `{label, re}` overrides.
- **tools**: `webrain_download` gains `engine=ytdlp` — downloads video/audio via
  the installed yt-dlp binary (HLS/DASH/.m3u8, playlists, age/cookie-bound
  media), with `audio_only`, `format`, and a full `args` passthrough
  (`--write-subs`, `--embed-thumbnail`, `--cookies`, `--proxy`, …). Single URL
  or batch (`urls[]`) on one tool.

### Fixed
- **core**: `solve_turnstile` used the ureq 2.x `.set()` API — renamed to
  `.header()` for ureq 3 (unblocked the workspace build).

### Changed
- **tools**: `webrain_download` is browsemind `download_many` — `urls[]` plus optional
  `filter_extension` (`.mp4`, `.pdf`, `.js`, …) to narrow a batch to one file type;
  returns a clear error when nothing matches. Batch extraction is
  `webrain_batch` `op=extract`. Now a combined download surface: `engine` defaults
  to `http` (streaming, backward-compatible) and switches to `ytdlp` for
  video/audio; the standalone `webrain_ytdlp` tool was folded in (removed).
- **core**: `webrain_media` network capture now also flags downloadable
  docs/archives (`.pdf` `.zip` `.doc(x)` `.xls(x)` `.ppt(x)` `.csv` …) so
  `download_many(urls=[...], filter_extension=...)` covers "download any file from
  network captures".
- **core**: `download_files` now streams responses to file via
  `Body::into_reader()` instead of `read_to_vec()`. The default 10 MiB body cap
  silently failed on multi-hundred-MB video files; large mp4s now download
  correctly and without buffering the whole file in memory.
- **core**: page-state responses made compact — `ELEMENTS_JS` capped at 60
  elements and visible text at ~3 KB (`PAGE_TEXT_CAP`), cutting `webrain_navigate`
  responses from ~40 KB to ~9 KB.
- **core**: `webrain_a11y` now emits `{role, name, value, css_path, xpath}` for
  each accessibility node (single `DOM.getDocument` walk). Interactive elements
  are index-only; precise selectors come from the a11y tree.

### Fixed
- **mcp**: `tools/call` responses were MCP-nonconforming — the raw tool payload
  (`{"status":...}`) was returned directly as `result`, so `result.content` was
  missing and clients threw "r.content is not iterable" on every tool call.
  Results are now wrapped in `{content:[{type:"text",text}]}` with `isError`
  derived from `status`.
- **core**: `Runtime.evaluate` landed in a stale pre-navigation execution context
  after `Page.navigate` (empty `document.body`, stub `performance`). The WS
  reader now tracks the default execution context from
  `Runtime.executionContextCreated` and passes `contextId` to evaluate.
- **core**: regex `url` pattern unterminated-char-literal build error.
- **core**: open-tab double-load (tab opened blank, then navigated once).

## [0.1.0] - 2026-07-31

### Added
- **core**: Rust CDP browser-automation agent. `webrain-core` defines one
  `BrowserBackend` trait over CDP WebSocket (`CdpBackend`) that drives Chrome,
  Edge, Lightpanda, or Obscura with the same code; `SessionPool` per-session
  isolation; `STEALTH_JS` anti-bot injection; default-execution-context tracking.
- **core**: page interaction — `navigate`, `evaluate` (arbitrary JS → JSON),
  `click`/`type`/`press`/`scroll`, `snapshot`, `get_html`, `get_images`,
  multi-tab (`open_tab`/`close_tab`), accessibility tree, overlay dismissal.
- **core**: capture — single + full-page `screenshot` (`webrain_screenshot`),
  PDF export, PixelRAG vision tiles (`webrain_pixel`), and a vision index with
  cosine `VectorStore` + embed `Endpoint` (`webrain_vision_index` / `retrieve`).
- **core**: extraction — `webrain_extract_json` (CSS-schema, zero-LLM),
  `webrain_extract_regex` (built-in patterns + custom `{label, re}`), and
  `webrain_eval` (JS → JSON).
- **core**: spider/crawl — BFS `SpiderEngine` (`webrain_spider`) and web search
  (`webrain_search`).
- **core**: batch + download — `webrain_batch` (fetch/extract/screenshot),
  `webrain_download` (streaming `Body::into_reader`, extension filter).
- **mcp**: `webrain-mcp` stdio JSON-RPC server owning the CDP connection, with a
  25-tool dispatch table (`webrain_eval`, `webrain_navigate`, `webrain_click`,
  `webrain_media`, `webrain_console`, …).
- **cli**: `webrain-cli` single `webrain` binary, `match`-based subcommands
  (no clap) mirroring the MCP tool surface.
- **workspace**: Cargo workspace (`resolver 3`, `edition 2024`, Rust 1.85) with
  `webrain-core` / `webrain-mcp` / `webrain-cli`; MIT license; docs/
  `ARCHITECTURE.md`; Keep-a-Changelog `CHANGELOG.md`; CI + release +
  changelog-enforce workflows.
