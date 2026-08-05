# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

_No unreleased changes yet._

## [0.3.3] - 2026-08-05

### Added

- **docs**: new `concepts/runtime-flow.mdx` — source-grounded runtime flow and
  architecture walkthrough (CLI entrypoint → MCP dispatcher → tool dispatch →
  CDP backend → engines/login/vault), with a request-flow diagram and a
  symptom→layer failure table.
- **docs**: `reference/tools.mdx` gains a tool-to-code map (each tool family →
  implementation file, browser vs no-browser, shared vs engine-specific).
- **docs**: `concepts/browsers.mdx` gains an engine capability matrix
  (Chrome / Obscura / Lightpanda / `fetch_http` across paint, screenshots,
  SPA interaction, challenges, a11y, batch, static-HTML speed).
- **docs**: light/dark logo variants (`webrain-logo-rounded` / `-nobg`) for the
  Mintlify theme + a GitHub icon in the navbar.

### Fixed

- **cli**: `webrain upgrade` now spawns the package-manager update (brew /
  scoop) detached and exits, so Scoop no longer sees the upgrade command
  itself as a running instance of webrain and refuses to replace the binary.

## [0.3.2] - 2026-08-05

### Added

- **docs**: new Mintlify docs site in `docs/` — overview, quickstart,
  installation, browsers/challenges concepts, structured-extraction /
  scrape-at-scale / auth-and-login guides, full 51-tool reference, CLI + env
  reference, deployment, troubleshooting, contributing. Built with the Mintlify
  CLI (`docs/docs.json`); existing `docs/*.md` files left in place, untouched.
- **docs**: changelog-map page (`docs/changelog.mdx`) — version-anchored
  history, linked in the Project nav.

### Changed

- **docs**: `quickstart.mdx` and README now show the full MCP client setup
  (stdio + HTTP transports, VS Code / Claude Desktop / Cursor configs,
  `webrain_guide` verification).
- **skills**: `skills/webrain/SKILL.md` upgraded to a full agent-skill contract -
  richer frontmatter, `When to use`, `Recommended limits & token discipline`,
  and a 7-step end-to-end `How to invoke` flow.
- **docs**: new `Prerequisites` section in `README.md` and the docs site.
- **docs**: ADR-001 graph note refreshed to 2026-08-05 state — 875 nodes /
  2287 edges, 3 entry points, new hotspots (`vault.get` fan-in 26,
  `ensure_page_attached`, `send_cmd_with`), 51 MCP tools (up from 34).
- **style**: `cargo fmt` on the mcp session-routing match indent.

## [0.3.1] - 2026-08-05

### Added

- **cli**: `webrain install --engine lightpanda` now downloads the lightpanda
  binary from the `lightpanda-io/browser` GitHub release (raw asset, no
  archive) into the engine cache, mirroring `--engine obscura`.
  `find_lightpanda()` also discovers the cached build, so `webrain lightpanda`
  just works after install. Lightpanda publishes no Windows binary — on Windows
  the install bails with the Docker fallback (`lightpanda/browser:nightly`)
  instead of silently installing Chrome.

### Changed

- **cli**: `webrain install --engine <unknown>` now errors with a clear message
  (`try chrome, obscura, or lightpanda`) instead of silently installing Chrome.

### Fixed

- **mcp**: `webrain_open_session(cdp_url=…)` now actually routes tools to that
  session — every browser tool accepts an optional `session_id` argument.
  Previously routing only worked via the `Mcp-Session-Id` HTTP header, so
  `open_session(cdp_url=obscura)` never switched navigate/batch/setcookies and
  everything kept hitting the default Chrome backend.
- **mcp**: a browser kill/restart no longer wedges the cached backend forever —
  dead-socket errors (`os error 10054` / connection reset / stream closed) now
  drop the backend so the next call reconnects fresh.
- **core**: the CDP WebSocket connect retries once with a longer budget (20s)
  after the initial 5s fail-fast, tolerating obscura's slow cold-start
  handshake.

## [0.3.0] - 2026-08-05

### Added

- **tools** `webrain_page_info` —
  just-in-time page context (viewport/page size, scroll position,
  pixels/pages above & below, position %) so the LLM knows when to scroll.
- **tools** `--state`/`--restore`:
  `webrain_save_state` / `webrain_restore_state` — export/import a profile's
  auth state (cookies + localStorage) to `state.json` so logins follow you
  across machines.
- **cli**: `webrain -v` / `--version` / `version` prints the version.

### Fixed

- **core**: `webrain install` failed with "the response body is larger than
  request limit" — ureq's `read_to_vec()` caps bodies at 10 MB, too small for
  the Chrome/Obscura engine zips. Now reads via the unlimited
  `into_with_config()` reader.

### Changed

- **docs**: rewrote README — removed the table of contents and made it
  install-first with a "Why webrain?" comparison and a use-case section;
  added the Scoop **extras** install option.
- **build**: moved the `Dockerfile` into `docker/` and added
  `docker/docker-compose.yml` (webrain MCP server + persistent
  vault/profile/cache volumes).
- **tools**: deduped the repeated JS→JSON parse chain in `tools.rs` into
  `parse_json_str` / `arr_len` helpers — no behavior change.

## [0.2.0] - 2026-08-05

### Added
- **cli**: `webrain upgrade` — updates to the latest release. Delegates to
  Homebrew/Scoop when installed through one (`brew upgrade webrain` /
  `scoop update webrain`), otherwise self-updates the running binary in place
  from the latest GitHub release.
- **install**: one-line installers — `scripts/install.sh` (Linux/macOS) and
  `scripts/install.ps1` (Windows) fetch the latest release binary per OS and
  put `webrain` on PATH:
  `curl -fsSL https://raw.githubusercontent.com/prokopis3/webrain/main/scripts/install.sh | bash`.
- **dist**: submitted to the official Scoop `extras` bucket (PR
  ScoopInstaller/Extras#18455) and published a Homebrew tap
  (`prokopis3/homebrew-webrain`; `brew tap prokopis3/webrain && brew install
  webrain`).
- **docs**: `CONTRIBUTING.md` (conventional commits + changelog-enforced PR
  policy), README badges (release/license/platforms/last-commit/stars),
  Quick Start + Traditional Selectors + Commands + Updating sections.

## [0.1.1] - 2026-08-04

### Added
- **docs**: real all-OS install one-liners (PowerShell + curl against release
  binaries) and the working scoop bucket (`prokopis3/scoop-webrain`).

### Fixed
- **ci**: the Linux release binary now builds on `ubuntu-22.04` (glibc 2.35) —
  the previous `ubuntu-latest` build required glibc 2.39, so `webrain-linux`
  failed with "GLIBC_2.39 not found" on Ubuntu 22.04 / Debian 12 and older.

## [0.1.0] - 2026-08-04

### Added
- **workspace**: Cargo workspace (`resolver 3`, `edition 2024`, Rust 1.85) with
  `webrain-core` / `webrain-mcp` / `webrain-cli`; MIT license; docs/
  `ARCHITECTURE.md`; Keep-a-Changelog `CHANGELOG.md`; CI + release +
  changelog-enforce workflows.
- **core**: Rust CDP browser-automation agent. `webrain-core` defines one
  `BrowserBackend` trait over CDP WebSocket (`CdpBackend`) that drives Chrome,
  Edge, Lightpanda, or Obscura with the same code; `SessionPool` per-session
  isolation; `STEALTH_JS` anti-bot injection; default-execution-context tracking.
- **mcp**: `webrain-mcp` stdio JSON-RPC server owning the CDP connection, with a
  25-tool dispatch table (`webrain_eval`, `webrain_navigate`, `webrain_click`,
  `webrain_media`, `webrain_console`, …).
- **cli**: `webrain-cli` single `webrain` binary, `match`-based subcommands
  (no clap) mirroring the MCP tool surface.
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
- **cli**: `webrain doctor` — full install diagnosis: version, MCP server, CDP
  ports (9222/9224/9225), engine discovery (chrome/lightpanda/obscura),
  encrypted vault, Python stealth sidecar, and a `recommend` line. Exit 0 when a
  browser is reachable. `--doctor` kept as an alias.
- **core/cli**: agent-browser-style engine install — `webrain install` downloads
  Chrome for Testing into a cache dir (`WEBRAIN_BROWSERS_DIR`) and that build
  wins discovery over system Chrome; `webrain install --engine obscura` downloads
  the latest Obscura release (`--stealth` picks the BoringSSL build).
  `webrain lightpanda` / `webrain obscura` spawn the CDP servers
  (`launch_lightpanda`/`launch_obscura`; binary from PATH / `~/.lightpanda` /
  `~/.obscura` / `~/.local/bin` / `WEBRAIN_LIGHTPANDA` / `WEBRAIN_OBSCURA`).
  Windows `.zip` via the `zip` crate; linux/macOS `.tar.gz` via system `tar`.
- **docs**: `README.md` with all-OS install (cargo / homebrew / scoop /
  from-source), engine + MCP tool guides, marketplace/MCP-client setup, and repo
  logo (`assets/webrain-logo.png`).
- **core/mcp**: secure `webrain_login` — fully-automatic login from a local
  encrypted vault. The server decrypts the secret in-process and injects it into
  the browser via CDP; the value never passes through the model, chat, or logs.
  `webrain_profiles` lists vault entries (names only). Optional TOTP (RFC 6238)
  auto-injection when a site gates with 2FA.
- **core/cli**: `webrain vault set|list|rm` — enroll credentials with hidden
  prompts (never argv/chat). AES-256-GCM vault at `%APPDATA%/webrain` or
  `~/.config/webrain` (`vault.json` index + 0600 `vault.key`), portable to any
  OS, no daemon. Optional TOTP seed at enroll.
- **core**: stealth hardening — `PluginArray`/`MimeTypeArray` rebuilt on the real
  prototype (a plain array is the classic detectable leak) with the standard PDF
  plugin names, `navigator.connection` (4g) stub, full `window.chrome`
  (app/csi/loadTimes) stub, `permissions.query` notifications reflection, plus
  CDP-level `Network.setUserAgentOverride` (real Windows Chrome 151 UA + Win32
  platform) and `Emulation.setAutomationOverride` on attach. Element snapshot
  redacts `input[type=password]` values.
- **core/mcp**: `webrain_download engine="ytdlp"` now works in the no-browser
  path too — it was silently forced onto the HTTP engine, so the advertised
  yt-dlp engine was dead over HTTP. One shared `engines::download_ytdlp`
  implementation serves both the stdio and HTTP transports.
- **core/mcp**: `webrain_spider` gains Scrapling `AutoThrottle` — adaptive
  per-domain delay tuned from observed latency (speeds up on fast servers,
  doubles on a blocked/error page, capped at `autothrottle_max_ms`, floored at
  `delay_ms`). Never guess a delay again.
- **core/mcp**: `webrain_spider` gains Scrapling `crawldir` checkpoint/resume —
  persists `{queue, seen}` every N pages; a later crawl with the same `crawldir`
  resumes from where it stopped. Checkpoint deleted on a clean (queue-drained)
  finish, kept when the crawl is capped/timed-out so resume continues.
- **core/mcp**: `webrain_spider` returns a `stats` block `{elapsed_ms,
  pages_ok, pages_err, page_ms_total}` — consistent with the batch stats block.
- **core/mcp**: `webrain_sitemap` tool — discover crawlable URLs from a site's
  sitemap (spider-rs `crawl_sitemap` / Scrapling `SitemapSpider`). Follows
  robots.txt `Sitemap:` → sitemap_index.xml → leaf sitemaps → every `<loc>`.
  Pure HTTP via the pooled agent, zero new deps (regex `<loc>` parse). Feed the
  returned URLs into `webrain_batch`/`webrain_spider` for a full crawl.
- **core/mcp**: `webrain_spider` gains Scrapling/spider-rs features:
  `allow`/`deny` URL regex filters (LinkExtractor `allow`/`deny`,
  spider-rs whitelist/blacklist), `retry` (re-fetch failed pages, 200ms backoff),
  `delay_ms` (polite crawl), and `crawl_timeout_secs` (hard wall-clock cap).
  Filters applied in the shared crawl loop — one spot covers every strategy.
- **mcp**: every tool response now carries `ms` (wall-clock elapsed) next to the
  existing `tokens` — per-tool-run latency + token cost at the one choke point
  (`with_token_cost`), both stdio and HTTP transports.
- **core/mcp**: batch results gain per-URL `ms` (tab-open→result wall-clock) and
  `webrain_batch` responses gain a `stats` block
  `{total, ok, errors, ms_total}` — the LLM sees at a glance which URL was slow
  and the whole run's cost, instead of counting result rows.

- **core**: `webrain_navigate`/`snapshot` now return a `links` field — deduped
  same-origin hrefs (≤200) via new `LINKS_JS`. One-call crawl/internal-link
  discovery (was: separate eval for hrefs).
- **core**: `webrain_batch(op=extract|interact)` results now carry a parsed
  `data` array (single-page `extract_json` shape) instead of a JSON string
  inside `text` (`text` kept for backward compat). Kills the data/text
  confusion an agent hits when tallying batch results.
- **docs**: agent guide + decision guide gain task-derived lessons — `/ajax`
  offset shortcut for load-more/infinite pages (fastest path, dedupe sliding
  windows), the async-eval-on-obscura null caveat (use `op=interact`), and the
  obscura Docker `--host 0.0.0.0` requirement.
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

### Changed
- **agent guidance**: `webrain_get_html` is now LAST RESORT. Tool description +
  AGENT_GUIDE rules + decision guide all instruct: never return raw HTML when
  `webrain_snapshot`/`clean`/`eval`/`extract_json`/`table`/`regex` give page
  text/structure cheaper. Only call `get_html` when the task explicitly asks
  for HTML markup, and remind the user why.
- **core**: pooled HTTP agent — `webrain_fetch_http`, `webrain_validate_urls`,
  `webrain_download` now share ONE `ureq::Agent` (static `OnceLock`) instead of
  building a fresh agent (new TCP+TLS handshake) per call. Keep-alive across
  calls makes offset/pagination probing ~0.3-1s faster each.
- **core**: `webrain_fetch_http` returns `content_type` + `bytes` and no longer
  truncates JSON responses (HTML still capped at 3000 chars), so a single probe
  can reveal a JSON `total`. Captures pagination headers (`x-total-count`,
  `link`, `content-range`, `x-next-page`) into `headers` when the server sends
  them — one-call count discovery instead of boundary-probing.
- **core/mcp**: `webrain_batch(op=extract|interact)` no longer mirrors the parsed
  `data` array into `text` as the same JSON string. Every extract batch previously
  carried the products TWICE (response bytes + LLM output tokens ~2×). `data` is
  the payload now; `text` stays empty for schema extract (interact keeps raw
  innerText in `text` only when no schema is set). Halves batch extract payloads.
- **core**: `apply_blocking` is a no-op for default `NavOpts` — the base
  `BLOCKED_URLS` is already set once at tab attach, so per-navigation re-sending
  the same 28 patterns was a redundant CDP round-trip per page.
- **mcp**: `webrain_a11y` filter is forgiving — `role` is a substring match
  (`button` finds `pushbutton`/`radiobutton`) and `filter` matches node name OR
  value OR css_path, so Material/Google controls whose label lives in a
  descendant are found. Description carries the ARIA role cheat-sheet
  (combobox/option/tab/radio…). Now emits `{role, name, value, css_path,
  xpath}` for each node (single `DOM.getDocument` walk); interactive elements
  are index-only; precise selectors come from the a11y tree.
- **agent**: decision guides (`AGENTS.md` + `docs/AGENT_DECISION_GUIDE.md`)
  codify the verified rules: Material/SPA interaction → real Chrome via
  `cdp_urls` (never obscura/lightpanda — no layout/paint engine); lightpanda
  `captureScreenshot` returns a fake placeholder PNG; extract from
  container/card-level DOM, not bare `$` text nodes.
- **core**: `launch_chrome`/`launch_lightpanda`/`launch_obscura` share one
  `spawn_and_wait` helper (port-open bail + 20s CDP wait + kill-on-drop).
- **tools**: `webrain_download` is browsemind `download_many` — `urls[]` plus
  optional `filter_extension` (`.mp4`, `.pdf`, `.js`, …) to narrow a batch to
  one file type; returns a clear error when nothing matches. Now a combined
  download surface: `engine` defaults to `http` (streaming, backward-compatible)
  and switches to `ytdlp` for video/audio; the standalone `webrain_ytdlp` tool
  was folded in (removed).
- **core**: `webrain_media` network capture now also flags downloadable
  docs/archives (`.pdf` `.zip` `.doc(x)` `.xls(x)` `.ppt(x)` `.csv` …) so
  `download_many(urls=[...], filter_extension=...)` covers "download any file
  from network captures".
- **core**: `download_files` now streams responses to file via
  `Body::into_reader()` instead of `read_to_vec()`. The default 10 MiB body cap
  silently failed on multi-hundred-MB video files; large mp4s now download
  correctly and without buffering the whole file in memory.
- **core**: page-state responses made compact — `ELEMENTS_JS` capped at 60
  elements and visible text at ~3 KB (`PAGE_TEXT_CAP`), cutting
  `webrain_navigate` responses from ~40 KB to ~9 KB.

### Fixed
- **core**: `with_crawl_timeout(0)` now means "no cap". Before, tools.rs passed
  `0` when the arg was absent → `Some(0)` → deadline = now → every spider crawl
  stopped before the first page (returned 0 pages). One guard in the shared
  builder fixed all callers.
- **core**: `Network.setBlockedURLs` now adapts to the backend's param shape —
  Chrome/obscura take `urls: [string]`, lightpanda takes
  `urlPatterns: [{urlPattern, block}]` (custom; lightpanda src/cdp/domains/network.zig).
  Tries standard first, retries with lightpanda's shape on `MissingField`, so
  tracker/resource blocking works on both engines.
- **core**: dropped the `exec_ctx` contextId tracking on `Runtime.evaluate`.
  Lightpanda fires a SECOND `Runtime.executionContextCreated` marked
  `isDefault=true` when a Turbo-style page re-renders into a new frame (FID-2),
  and that context is empty — the reader cached it and every later eval hit a
  blank page. Callers already wait for interactive/complete before extracting,
  so the browser default context is live; no-contextId eval works on both
  engines (verified live on obscura + lightpanda).
- **core**: `webrain_batch` now detects single-target backends and falls back to
  sequential single-tab reuse. Lightpanda `serve` holds ONE browser context and
  its 2nd `Target.createTarget` errors `TargetAlreadyLoaded`
  (src/cdp/domains/target.zig) — parallel tabs are impossible by design. A raw
  CDP probe (`single_target_probe`) distinguishes it from obscura/Chrome
  (multi-tab parallel), which keeps its parallel path untouched. Also handles
  the "a target is already open from a prior navigate" case by reusing it.
- **core**: `webrain_type` index mismatch — the index now uses the SAME selector
  as `ELEMENTS_JS`/`click` (`a, button, input, select, textarea, [role=button]`),
  so snapshot/navigate indices map 1:1 to `type_text`. Before, `type_text`
  enumerated only `input/textarea/select`, so on pages with a leading link/button
  (e.g. the scrapingcourse CSRF login: `#logo-link` first), the index pointed at
  the wrong field (typed into password instead of email). Guard also rejects
  non-input targets.
- **mcp**: `tools/call` responses were MCP-nonconforming — the raw tool payload
  (`{"status":...}`) was returned directly as `result`, so `result.content` was
  missing and clients threw "r.content is not iterable" on every tool call.
  Results are now wrapped in `{content:[{type:"text",text}]}` with `isError`
  derived from `status`.
- **core**: `Runtime.evaluate` landed in a stale pre-navigation execution context
  after `Page.navigate` (empty `document.body`, stub `performance`). The WS
  reader now tracks the default execution context from
  `Runtime.executionContextCreated` and passes `contextId` to evaluate.
- **core**: `solve_turnstile` used the ureq 2.x `.set()` API — renamed to
  `.header()` for ureq 3 (unblocked the workspace build).
- **core**: regex `url` pattern unterminated-char-literal build error.
- **core**: open-tab double-load (tab opened blank, then navigated once).

### Removed
- **core**: dead SHA-256 crawl disk cache (`cache_read`/`cache_write`) — zero
  callers (ponytail-audit).
- **core**: unused `PageResult.screenshot_b64` field and dead `lib.rs` re-exports
  (`EmbedInput`, `VectorStore`).
- **core**: unused in-process `obscura` git dependency + `obscura` feature —
  replaced by the `webrain install --engine obscura` binary path.
- **tools**: standalone `webrain_ytdlp` tool — folded into `webrain_download`
  (`engine=ytdlp`).
- **scripts**: one-off `scripts/merge_task2.ps1` data migration.
