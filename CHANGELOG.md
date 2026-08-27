# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

_No unreleased changes yet._

### Changed

- **deps**: bump `futures-util` 0.3.33 → 0.3.34.

### Fixed

- **cli**: `webrain vault user` no longer echoes the username back to the
  terminal (CodeQL cleartext-log — a username is half a credential).

## [0.8.0] - 2026-08-27

### Added

- **core/mcp**: `webrain_batch` gains `op=markdown` — full page HTML →
  Markdown via pure-Rust `htmd` (turndown.js-equivalent, skips script/style/
  noscript), surpassing `fetch`'s 3000-char `innerText` cap. Optional `query` +
  `top_k` bm25-prune the markdown to query-relevant chunks (crawl4ai
  fit_markdown style, reuses existing `bm25_filter`, zero LLM).
- **core/mcp/cli**: `webrain_serp` / `webrain serp` reply includes a
  `per_engine` breakdown — `[{engine, status: ok|empty|skipped, count}]` —
  showing which engines contributed results and which were skipped (CLI prints
  `engines: duckduckgo=ok(5), bing=empty(0), ...`).

### Security

- **core**: engine archives are extracted with a zip-slip guard — absolute
  paths and `..` traversal entries are rejected, and Unix exec bits from the
  archive are preserved (`chrome`/`whisper-cli` stay executable on Linux).
- **core**: `lightpanda`/`obscura` CDP servers now bind to `127.0.0.1`
  (previously `0.0.0.0` exposed unauthenticated browser control on the LAN);
  Chrome already bound loopback.
- **core**: the 2captcha submit is now a form POST — the API key and proxy
  credentials no longer travel in the URL query string (access-log exposure).
- **core**: vault hardening — `vault.key` is created exclusively with `0600`
  (no world-readable window, no concurrent-run clobber), unreadable keys/index
  errors are propagated instead of treated as "empty" (no silent data loss),
  and `vault.json` writes are atomic (temp + rename).
- **cli**: `webrain upgrade`/`self-update` now verify the downloaded binary's
  SHA-256 against the release's `checksums.txt` and fail closed on mismatch.
- **mcp**: HTTP transport caps the request body (16 MiB) — a huge client
  `Content-Length` can no longer exhaust memory — and `port` args are validated
  with `u16::try_from` instead of silently truncating.
- **installers**: `install.sh`/`install.ps1` download to a temp file, reject
  empty downloads, and atomically move into place (an interrupted download
  can no longer destroy a working install).
- **mcp**: search queries are percent-encoded (space→`+` let `&`/`#`/`%`
  inject parameters or truncate the query), and the `webrain_download`
  `isError` flag reflects the result instead of being hardcoded `false`.
- **cli**: `webrain serp` errors go to stderr with a non-zero exit (scripts
  can detect failures; `--json` stdout stays pure JSON), launch diagnostics
  are suppressed under `--json`, the screenshot filename gets sub-second
  precision, and the PowerShell stop-command escapes `'` in the exe path.
- **installer/port script**: `port_blocklist.ps1` validates hostnames per RFC
  1123, guards against clobbering the blocklist on a truncated response
  (<1000 domains → abort), writes BOM-less UTF-8 (PowerShell 5.1's BOM broke
  `include_str!` exact matches), and retries with TLS 1.2.
- **core**: the TOTP base32 decoder rejects seeds with non-zero trailing bits
  (a malformed key is an error instead of a silently wrong code), and the
  template filler uses two-phase sentinel replacement so a literal `SEL`/`VAL`
  in real values can't corrupt the fill.
- **ci**: third-party GitHub Actions are pinned to full commit SHAs (the
  mutable `stable`/`v2`/`v3`/`v6` refs could be force-moved); Dependabot keeps
  the SHAs updated. `dependabot.yml` no longer references the nonexistent
  `dependencies` label (Dependabot won't create it — the PRs were silently
  skipped or unlabeled).
- **installers**: `install.sh`/`install.ps1` verify the downloaded binary
  against the release's published `checksums.txt` before installing — a
  tampered release/mirror/MITM fails the check instead of silently installing
  arbitrary code (release.yml already publishes the checksums).

### Docs

- **docs/landing**: coverflow cards render in the correct order (`--card`
  indices were inverted), hovering the coverflow pauses the transcript groups
  too (not just the cards), the no-JS fallback shows the benchmark comparison
  via a `--fill` custom property, and `bar-fill` reads `var(--fill, 0)`.
- **docs/landing**: `runPlayground`'s async continuations are generation-
  guarded (a stale run can no longer reveal a card after reset), the
  IntersectionObserver callback is idempotent (no double-reveal flash), the
  no-IO fallback no longer flashes content hidden→shown, and the install-tab
  demo stops auto-cycling once the user clicks.
- **docs/landing**: `h1 .w` `will-change` is scoped to `prefers-reduced-
  motion: no-preference`, and `.try-input` gets a `:focus-visible` ring.
- **docs/logo-nav**: the injected script appends to `document.head` (with a
  `documentElement` fallback), the nav bindings react to keyboard (`Tab`
  counts as a user click so the poll doesn't steal focus), and the loader
  bails out early once the block is settled.
- **docs/config**: commitlint's type/scope overlap is documented as deliberate
  (the `.md`-scope templates and the `docs` type intentionally overlap) so a
  future reader doesn't "fix" it (#7).
- **docs/script**: `port_blocklist.ps1` documents its alphabetical crop to the
  3500-slot blocklist cap (#27).

### Fixed

- **core**: `bm25_filter` document-frequency now counts distinct documents per
  term, not raw occurrences — a term repeated inside one doc could inflate
  `df` past `n/2` and flip IDF negative, penalizing the very chunks that
  matched the query (hurt `op=markdown` pruning and `extract` bm25 mode).
- **core**: `build_clean_js` selector list uses a leading comma — the default
  (`exclude_social=false`) previously emitted a trailing comma that made
  `querySelectorAll` throw a SyntaxError, breaking the clean-text path.
- **core**: `batch_map`/`pdf_extract_batch` return early on empty input instead
  of panicking (`urls[0]` index-out-of-bounds / `chunks(0)`).
- **core**: CDP backend — consistent `active → tabs` lock order (removes a
  deadlock inversion between `register_tab` and `active_session`/`close_tab`),
  and a 30s response timeout so one unanswered command can't hold the
  connection mutex forever.
- **core**: `launch` reaps the killed child (`wait()` in `Drop`) so long-lived
  parents no longer accumulate zombies, and `install_chrome` finds the macOS
  "Google Chrome for Testing" executable name.
- **core**: `install` `.ok` markers are only trusted when the destination file
  still exists, the download progress loop terminates when all workers finish
  (a permanently-failing chunk no longer spins forever), and `last_pct` is
  actually updated.
- **mcp**: the guest Chrome launch guard is only `mem::forget`-ed after a
  successful CDP attach — a failed attach now kills the guest instead of
  leaking an orphan.
- **cli**: `webrain fetch` truncates text on a UTF-8 char boundary
  (`floor_char_boundary`) instead of panicking on non-ASCII pages.
- **core**: `video` — `Detail::Transcript`/`max_frames:0` now produce zero
  frames (the `.clamp(1, …)` made them impossible), the uniform-fps fallback
  honors `start`/`end` (`-ss`/`-to`), ffmpeg exit status is checked, the WAV
  payload is labeled `audio/wav` (not `audio/mpeg`), and `watch_batch` gives
  each worker a distinct work dir (concurrent runs clobbered the same
  `watch_<pid>` dir) and recovers from a poisoned mutex.
- **core**: `serp` — `brave` removed from `HTTP_ENGINES` (it's a browser-only
  SPA that plain-HTTP always parsed to zero), `serpapi_google` sends the
  `start` offset for pagination, protocol-relative `//host` links resolve
  correctly (not `google.com//host`), and the post-browser serpapi fallback
  honors `opts.fallback`.
- **core**: `serp` — **auto is relevance-filtered**: results sharing no
  significant query token with the query are dropped at the source (per-page in
  `http_search`/`browser_search` and at the merge), so a junk page from a
  flagged/GeoIP'd IP (bing's unrelated "results" for "tokio rust") can no
  longer fill `limit` ahead of duckduckgo's real results; `auto` merges
  `duckduckgo → bing` (ddg first) and HTTP-fetches only those two — google/
  brave join the merge via the browser **only when a CDP engine is attached**
  (sequential — one browser, one active tab, and Google walls simultaneous
  /search requests; 20s-capped each; never HTTP-polled pointlessly) and are
  reported `skipped` otherwise; a failed/empty single engine now reports the
  fallback winner as its source.
- **core**: `serp` — **google's URL is now a real-browser URL**: `num` and `lr`
  (language-restrict) are gone — google answers those with an empty "did not
  match any documents. Reset search tools" page on automated/flagged sessions —
  and `start` is omitted on page 0 (canonical). That soft-block is now detected
  as a wall so the retry loop fails fast instead of burning 4 attempts on an
  empty page. Results are honest about provider: a fallback reply carries
  `source` (actual provider, e.g. `engine: google, source: duckduckgo`) in the
  JSON, CLI, and MCP envelope, so substituted results can never be mislabeled.
- **core**: **consent dismissal can no longer click a search result** — Google's
  app shell `#yDmH0d` (present on every google page) was in the consent-gate
  selector, and the AX last-button fallback then clicked an arbitrary result
  link on dialog-less pages ("consent=clicked:Tokyo provides an asynch" — the
  tab navigated to YouTube mid-search → parse 0). The gate now requires a real
  dialog/banner container, and the blind last-button fallback fires only when a
  strict overlay (role=dialog / aria-modal / consent form / onetrust) matched.
- **core**: `serp` HTTP engines retry a 2xx page that parses to zero results —
  engines under rate-limit pressure (bing intermittently) serve an empty /
  JS-shell page with HTTP 200, which used to short-circuit straight to
  "0 results → provider fallback" on the first try. The shared
  `http_search_page` now retries within the existing budget before giving up,
  so a transient empty page doesn't immediately mislabel the engine as broken.
- **core**: `serp` google pagination budgets pages on ~7 results/page instead
  of 10 — google serves ~6-7 organic results per page once video carousels /
  AI overviews / featured snippets crowd the SERP, so `--limit 20` used to
  stall at ~13 (2 pages × 6.5). Ceiling division now fetches enough pages to
  reach the limit (20 → ~19, 50 → ~50).
- **core/mcp/cli**: `serp` bing is now browser-backed like google/brave —
  bing's plain-HTTP endpoint caps at ~10 results/page and ignores `first` on
  rate-limited/GeoIP'd IPs (anomaly page or same-page repeat), so bing now
  routes through the real-browser backend (`first=1,11,21…` pagination) and
  auto-launches/attaches Chrome like the other browser engines. On an
  unblocked IP bing paginates; on a locked IP it still returns the real page-1
  results and falls back cleanly.
- **core/mcp/cli**: `serp` bing falls back to pure HTTP when no Chrome can be
  launched (verified on WSL Linux — `failed to spawn Chrome` no longer a hard
  error for bing; google/brave still require a browser). The Linux-only
  dead-code warning on `cmd_exists` is gone (cfg-gated to its macOS/Windows
  callers).
- **core**: `serp` bing's browser path makes ONE attempt (not 4) — an empty
  bing page usually means "no results for this query/IP" rather than a
  transient wall (the HTTP fallback retries those), so a no-result bing query
  now falls back to the chain in ~6s instead of burning ~30s in the
  google/brave wall-clearing loop.
- **core/mcp/cli**: OCR-review findings — (1) MCP `webrain_serp` bing now runs
  the pure-HTTP path directly when no Chrome can be attached/launched (the
  shared dispatch previously re-attempted the browser and hard-errored);
  (2) the google/auto no-stealth flag is scoped to the serp call and cleared
  after, so it no longer silently disables stealth_js for later browse/batch/
  scrape calls in the same MCP session; (3) CLI sets `WEBRAIN_NO_STEALTH` for
  the default `serp` invocation and case-variant/`--engine=` spellings, not
  just literal `--engine google|auto`; (4) the guest-Chrome launch message is
  `json_out`-gated (pure-JSON contract); (5) zip extraction rejects Windows
  drive-prefixed absolute paths (`C:/…`) that `Path::join` treats as absolute;
  (6) launch readiness falls back to a port-open probe for non-Chrome engines
  (obscura/lightpanda have no HTTP `/json/version`).
- **docker**: the release build no longer loses the binary — the cargo `target`
  cache mount is ephemeral (its contents never reach the image layer), so the
  binary is now copied out within the same `RUN`, and the target cache is keyed
  by `$TARGETARCH` so multi-arch builds don't mix amd64/arm64 artifacts.
- **core/cli**: the three `if let … && let …` chains (introduced by the clippy
  auto-fix) are rewritten as nested `if let` — let-chains need Rust 1.88 but
  the workspace MSRV is 1.85; the collapsible-if lint is allowed at the one
  retained site with an explanatory comment.
- **core/scripts/ci/docs**: second OCR-review batch — (1) `vault` retries the
  loser-side key read (transient 0-byte/partial file on create-race no longer
  hard-errors); (2) robots.txt `Disallow` matches path+query (RFC 9309); (3)
  2captcha poll is POSTed (key never in a query string); (4) `cdp` close_tab
  holds both locks for the active reassignment (no race), Enter on a form
  dispatches a submit event first (SPA `onSubmit` runs, native submit only if
  not canceled), and print/screenshot get a 120s budget instead of the flat
  30s; (5) `watch` workers get per-worker `out_dir` subdirs even under an
  explicit base, and the ffmpeg fallback uses `-t` (duration) instead of
  `-to` before `-i`; (6) engine-cache version ordering parses the full dotted
  version (`0.9.0` < `0.10.0`) and mac CfT search depth is 6; (7) CLI serp
  error returns `Err` (drops the `--fresh` Chrome) instead of `exit(1)`;
  (8) `check_docs_orphans.py` actually validates `.mdx` files; (9) `release.yml`
  keeps the changelog H1 at the top and tolerates a rejected changelog push;
  (10) `build-skill.sh` pipefail-safe grep, correct pathspec, SKILL.md presence
  check, dist exclusion, and repo-guard ordering; (11) `install.sh`/`install.ps1`
  reject non-executable (HTML-200) downloads (ELF/Mach-O + MZ magic) and clean
  up temp files on any exit; (12) docs landing JS honors keyboard tab selection,
  stops the parallax rAF/listeners when the view is gone, and bails stale
  streamLines runs via the generation token.
- **core/mcp/cli**: `serp` `limit` raised **1..=50 → 1..=100** (pagination
  budget 5 → 10 pages; serpapi already honored 100).
- **core**: `login` — placeholder substitution is single-pass (a username
  containing "PASS" no longer gets corrupted), a missing captcha token bails
  with `waiting_for_human` instead of submitting a guaranteed-403 empty token,
  and the 2FA response reports `totp_filled`.
- **core**: `launch` — the CDP wait loop surfaces an early child exit via
  `try_wait()` (no more misleading 20s timeout), and `service`/`profile` names
  are validated so `/` or `..` can't escape the profiles root.
- **core**: `install` — DLL/backend copy errors propagate (silent broken
  installs are gone), cached engines sort by numeric version (`chrome-99` <
  `chrome-100`), and each download chunk has a 60s timeout.
- **core**: `vision` — embedding responses validate count/values (no more
  panic on `vecs[i]` or silent `0.0` garbage), an empty embed result errors,
  and the vector/caption store is mutually exclusive per id with atomic saves.
- **core**: `crawl` robots.txt now matches the URL path (absolute links were
  silently exempt from `Disallow`), and `reload_hard` drops the non-standard
  `location.reload(true)` boolean.
- **cdp**: the JS Enter fallback submits the form natively (`el.form.submit()`),
  and consent-button quads use `backendNodeId` (the old `nodeId` call always
  failed, so consent dialogs were never clicked).
- **mcp**: `webrain_eval_in_frame`, the batch `op=eval` `js` param, and the
  `drag` x1/y1/x2/y2 params are now in `list_tools` (previously
  undiscoverable to schema-driven clients).
- **core**: `throttle_tick` computes the plain average (the min/max/floor
  clamp chain could produce non-monotonic delays).
- **core**: `visible_text_len` skips `<script>/<style>/<noscript>` bodies
  (their raw JS/CSS no longer inflate the visible-text budget).
- **core**: engine downloads use a per-call unique temp name (concurrent
  installs can't clobber each other's `.part` files) and clean up sidecars.
- **core**: `detect_chrome_error` requires a corroborating interstitial phrase
  for body-only error codes (a title that merely contains e.g. `ERR` is no
  longer misread as a Chrome error page).
- **mcp**: `webrain_watch` runs off the executor thread (`spawn_blocking`) — a
  long watch no longer stalls other MCP requests, and the in-flight session
  table is capped at 64 sessions (evicts the oldest instead of leaking).
- **cli**: `webrain setcookies` accepts both the bare cookie array and the
  wrapped `{"count": N, "cookies": [...]}` shape.
- **core/cdp**: the captcha widget-claim JS returns the result object directly
  — it ended with `JSON.stringify(...)`, so `eval_js` (returnByValue) yielded a
  String and the cross-origin CDP click on the claimed captcha iframe was dead
  code (only the in-page `#cf-turnstile` fallback ran). The trusted iframe
  checkbox click now actually fires.
- **core/cdp**: `ELEMENTS_JS` emits a unique selector per element — id/class
  are CSS-escaped, and id/class-less elements get a body-relative
  `nth-of-type` chain instead of a bare tag name (a bare `input` resolved via
  `DOM.querySelector` to the FIRST match, so `set_file_inputs` could set files
  on the wrong node).
- **core/cdp**: the network block list is re-applied on every navigation —
  `Network.setBlockedURLs` replaces the session list, so a page navigated with
  `disable_resources`/`block_trackers` previously left those patterns sticky
  for every later default navigation.
- **core**: `detect_antibot` now requires TWO corroborating markers for generic
  phrases (`forbidden`, `access denied`, `just a moment`) — a support article
  that merely mentions one is no longer misread as a blocked/challenge page;
  and `crippled` is gated on element-extraction success (an empty `elements`
  from a failed eval can't mislabel a healthy page as a bot-limited shell).
- **core**: the 2captcha proxy is validated up-front (an unparseable URL, a
  missing port, or an unknown scheme fails loudly instead of silently dropping
  the proxy and wasting a paid solve), credentials are percent-decoded, and
  `socks4`/`socks4a` map to SOCKS4 (were mislabeled SOCKS5).
- **core**: `vault::set`/`set_username`/`remove` serialize the read-modify-write
  cycle with an in-process mutex — two concurrent mutations can no longer read
  the same snapshot and have the last `save_entries` drop the other's entry.
- **core**: `launch` waits for the CDP signature (`GET /json/version` →
  `webSocketDebuggerUrl`), not just an open TCP port — a foreign listener that
  grabs the port between check and bind can no longer be mistaken for the
  spawned engine.
- **core/cdp**: the first-attach path is serialized (two concurrent
  `ensure_page_attached` calls could each attach a separate page target,
  creating a duplicate tab and repointing `active`), and `click`/`type_text`/
  `hover` surface a stale-element error instead of a silent no-op.
- **core**: `crawl` fetches robots.txt off the executor with the 30s-timed
  pooled agent — a hung robots server can no longer stall the async worker
  indefinitely (a bare `ureq::get` had no timeout).
- **mcp**: `http_fetch` (fetch/search), `pdf_images`, and `vault::list` run off
  the executor (`spawn_blocking`); serp's `http_search`/`serpapi` and vision's
  `ask_viewport` provider chain are likewise off-executor (up-to-120s blocking
  HTTP no longer ties up a tokio worker).
- **mcp**: serp google's stealth-off is now a per-backend `no_stealth` flag on
  `CdpBackend` — the old `std::env::set_var("WEBRAIN_NO_STEALTH")` from a
  concurrent handler was an edition-2024 data race and permanently disabled
  stealth for every session in the process.
- **core/cdp**: `ELEMENTS_JS`/`LINKS_JS` now live on the `BrowserBackend`
  trait (`browser.rs`) instead of inside the CDP backend — the shared
  `snapshot` path no longer reaches into `backends::cdp`, so any backend can
  supply its own extraction script and the crippled gate sees the trait's
  contract (#38).
- **core**: `login` gate/captcha probe failures are observable — the evaluate
  result is matched and the error is logged (`tracing::debug!(error=…)`)
  instead of being silently dropped behind an `unwrap_or(false)` (#46).
- **core**: `vision::index_current_page` runs the blocking embed/store off the
  executor (`spawn_blocking`) — a large page no longer stalls other MCP
  requests on the tokio worker (#99).

### Changed

- **core/mcp/cli**: clippy `--workspace --all-targets` is clean (zero warnings)
  — doc-comment list/continuation fixes, `sort_by_key` for engine discovery,
  explicit `.truncate(false)` on the pre-allocated `.part`, `VectorStore::is_empty`,
  `#[allow(clippy::too_many_arguments)]` on the two 8-arg public APIs, and the
  `surface_tests` module moved to the end of `tools.rs` (items-after-test-module).
- **docker**: base images pinned to concrete minors (`rust:1.85-alpine`,
  `alpine:3.21`), the build copies only the workspace manifests + crate
  sources instead of the whole repo, and BuildKit cache mounts reuse the
  cargo registry + target dirs across builds. (Running as non-root is
  deferred — Alpine Chromium needs `--no-sandbox`, which `launch.rs` doesn't
  pass yet.)
- **cargo**: workspace tokio narrowed from `features = ["full"]` to the
  feature set the crates actually use (rt, net, time, io-util, sync, macros,
  process, signal, fs), and the direct `lopdf` pinned `0.44` → `0.41` to match
  `pdf-inspector`'s transitive version — one PDF stack instead of two (the
  extraction code already targeted 0.41's API).
- **ci**: PR-lint runs on `pull_request_target` (fork PRs get a read-only
  `GITHUB_TOKEN` under `pull_request`, so the commit-status + sticky-comment
  steps failed with 403 — safe here, no PR head code is executed); the release
  workflow gains a concurrency group + job timeouts; `ci.yml` gains a docs
  orphan check (`.github/scripts/check_docs_orphans.py`) that enforces the
  `.mintignore` promise — every `docs/` `.md`/`.mdx` must be in `docs.json`
  navigation or listed in `.mintignore`.
- **cli**: `webrain upgrade` is one careful cross-platform flow — Homebrew /
  Scoop delegate to the package manager detached (this process exits so it
  never holds the binary); raw installs self-update in place. Running
  instances handled per-OS: Windows stops only same-exe siblings (never an
  unrelated `webrain*` process or a second install), Unix swaps atomically
  and warns that a still-running server keeps the old version until restart.

### Docs (landing + README + guides)

- **docs**: landing gains a cosmic starfield background (galaxy, re-tuned from
  a stars.js concept to the electric-cyan/OLED palette) — twinkling white +
  cyan stars, three slow orbit "node" dots, a rare shooting star, masked to
  fade below the fold. Pure CSS motion (opacity/rotate) gated behind
  `prefers-reduced-motion: no-preference`; built from DOM spans so the
  Mintlify custom-page sanitizer keeps it (a `<canvas>` gets stripped); never
  runs on phones; self-healing like the rest of the motion layer.
- **docs**: the SERP showcase is now capability-honest (per the SERP claims
  review): the demo runs `engine=duckduckgo` (which genuinely reproduces the
  shown results over plain HTTP) instead of claiming `auto` fetches google in
  parallel; the "3 engines, no browser at all" stat is now "2 browserless by
  default (duckduckgo · bing)"; the copy notes `auto` is duckduckgo + bing in
  practice.
- **docs**: `AGENT_DECISION_GUIDE` SERP row fixed — `engine=auto` fires the
  HTTP engines (ddg · bing · google · brave) concurrently; google/brave only
  use the browser path when requested explicitly (`engine=google`/
  `engine=brave`), matching the code (was: "google joins via the browser
  path" for auto, which is not implemented).
- **docs**: README tool table now lists all **16** tools (added `webrain_serp`),
  plus a structured-search feature bullet, `webrain serp` CLI entry, and a
  duplicate Docker-prereq line removed; landing "fifteen" → "sixteen".
- **mcp**: the embedded `AGENT_GUIDE` (served by `webrain_guide`) now says
  **16 TOOLS** and drops `webrain_eval_in_frame` from the numbered list —
  it's a legacy dispatch-only executor that was never registered in
  `list_tools()`, so the guide now matches the actual `tools/list` surface.
- **docs**: landing gains a "skills & recipes" section (the in-repo
  `skills/webrain/` playbook: router skill, workflows, prompts, references)
  and a 6th real-run playground preset ("Real run · drone build" — a 1m43s
  DeepSeek session: `webrain_serp` discovery → `webrain_batch` 6-source fetch
  → synthesized build guide); the capabilities-bento stat "15 MCP tools" is
  now "16" (the last stale count).
- **docs**: P0 marketing-honesty pass — the "16 intent-based tools" cell now
  shows all 16 chips (added the missing `serp`); the "63 legacy one-action
  aliases" claim (the code has 53 dispatch arms / 16 registered) is now
  number-free and truthful; hero lede "beat captchas" → "clear bot gates"
  (native path clears non-interactive gates; interactive ones need a human);
  playground demos labeled illustrative (drone = the real 1m43s run).
  Art-directed design references for Hero / Proven on real jobs / the
  capabilities grid saved under `design/landing/` (not yet wired into the
  page — they match the live tokens).
- **docs**: the "Proven on real jobs" section gains a "Real operations, end
  to end" block — two full agent jobs as bento cards: the live skroutz.gr
  hardware-buy (16 tool calls · 0 challenges · 100% price match · ~$0.04,
  verdict Acer Helios 16 AI @ 3.699 €) and the multi-source drone-build
  research (2× serp · 6 sources · 1m43s · 12-part plan). Reuses the existing
  `.shell` scroll-reveal, so they animate with the same anime.js motion as
  the rest of the page.
- **docs**: landing "massive powerful" pass (v4.3) — hero gains a living
  neural-web spider network behind the terminal coverflow (sanitizer-safe
  `<path>`/`<rect>` rings + spokes + glowing nodes, two packets travel the
  rings via `anime.path`, reduced-motion keeps a static hairline
  constellation); each hero workflow terminal now carries the brand-identity
  TOOLS rail (`navigate`…`vision`, lit per transcript, JS-injected,
  self-healing); the skills section moves from a fourth bento to an
  asymmetric "router hub" layout (prompt → decide → browser/extractor
  route-map with traveling packets, three cells stacked beside it); the CTA
  band gains a converging particle stream (pure CSS, reduced-motion-safe).
  Copy pre-flight: zero visible em/en dashes (Mintlify smartypants turns
  `--op` into `—op` — the CLI command is now rendered as a JSX string), and
  metadata lines rationed to ≤1 middle-dot.
- **docs**: docs-accuracy audit against the 16-tool surface — `quickstart`,
  `mcp/cursor`, `guides/agent-decision-guide`, and `concepts/runtime-flow`
  stale "15"/"63-tool" counts → 16 (added the missing `webrain_serp` row to
  the agent-guide table); `reference/cli` drops the non-existent
  `webrain snapshot` example and the false click-with-`selector` claim
  (click is index-based; selectors live on `observe what=html` / `extract
  mode=schema` / `interact action=wait`); `reference/environment` adds
  `SERPAPI_API_KEY`, `WEBRAIN_2CAPTCHA_KEY`, `OPENROUTER_API_KEY`,
  `WEBRAIN_NO_STEALTH`, `WEBRAIN_STEALTH_NOISE`, and `EMBED_MODEL` /
  `EMBED_API_KEY`; `AGENT_DECISION_GUIDE` tool dialect modernized to the
  consolidated names (legacy one-action names still dispatch); unreferenced
  `benchmark-art.png` + `mcp-browser-demo.gif` removed; `.mintignore`
  completed with `IMPROVE_PLAN.md` + `SERP_CLAIMS_REVIEW.md`.

## [0.7.3] - 2026-08-15

### Fixed

- **core**: SERP results decode as UTF-8 — `serp_http_get` now reads raw bytes +
  `String::from_utf8_lossy` instead of ureq's charset-header-driven
  `read_to_string`, which mis-decoded (latin-1) every non-ASCII char into
  mojibake (`–` → `ΓÇô`). All engines serve UTF-8, so the lossy decode is
  always correct; API/JSON consumers now get clean text (a legacy console/pipe
  may still render UTF-8 as glyph soup — use `chcp 65001` or a UTF-8 terminal).
- **core**: no more transient Chrome on HTTP serp runs — removed the
  `chrome --version` UA probe (`chrome_ua_version` + `parse_chrome_ver`) that
  spawned a visible Chrome window (with crashpad children) on every
  duckduckgo/bing/auto invocation. UA/sec-ch-ua now use a static, internally
  consistent `CHROME_UA_VER` constant: the probe's "real" version never matched
  webrain's engine (Chrome for Testing, not system Chrome), and the HTTP path's
  rustls TLS is identifiable regardless of UA. Also speeds up every no-browser
  SERP call (no spawn + 5s watchdog).
- **core**: serp **brave pagination fixed** — Brave's `offset` is page-indexed
  (0, 1, 2, ...), not result-indexed: its own Next link is `offset=1` from
  `offset=0`. The old `offset=page*10` (e.g. `offset=10` for page 1) landed on a
  nonexistent page that Brave answers with a "no results" page, so `fresh==0`
  stopped the loop at one page (21 results). Now `offset=page`; verified:
  `--engine brave --limit 50` returns 50 results across 3 pages (fresh 21+20+15).
- **core**: serp debug logs name the real engine and brave skips the consent
  poll — the shared google/brave `browser_search` path hardcoded "google" in
  the `consent dismiss` / `serp page` trace labels (misleading under
  `--engine brave`), and it ran `dismiss_google_consent` (≤1.2s poll) on a
  path that never shows a consent modal. Logs now emit `engine=brave` /
  `engine=google`, and consent dismissal runs for google only.
- **mcp**: removed dead `legacy_tool_schemas()` — ~748 lines of
  `#[allow(dead_code)]` JSON schema data superseded by the consolidated
  16-tool surface (`tools/list` already returns `list_tools()`; legacy names
  still dispatch via `map_surface()` → executor arms, which are untouched).
  Tool surface and dispatch are byte-identical.
- **core**: serp/engines comments trimmed to `ponytail:`-style one-liners
  (CHROME_UA_VER, brave offset, consent gate, brave-offset test).

### Added

- **core**: serp **google guest flow = direct search URL + pagination** — the
  browser path navigates straight to `engine_url`'s `/search?q=..&start=..&num=..`
  (no homepage→type→submit) and merges `start=(page*10)` pages exactly like
  `http_search`, so `--limit 20` / `--page 2` return more than one 10-result
  page (dedupe + renumber + truncate). Walled `/sorry` pages add nothing → stop
  early → the existing retry/fallback chain takes over.
- **core**: SERP **consent-dismissal latency fix** — `consent_button` now runs a
  cheap DOM `querySelector` gate (known consent containers) before the expensive
  `Accessibility.getFullAXTree` walk, and `dismiss_google_consent` fast-polls
  (150ms, ≤1.2s) firing the TRUSTED click the instant the overlay renders.
  Previously a no-consent page paid 12 × getFullAXTree ≈ **42s**; now ~1.2s max.
  Also cut the google results wait beat to 1.2s (navigate already waits for
  interactive). Verified: `serp --engine google --limit 10` ≈ **6s / 10 results**
  (was ~45.5s).
- **core**: serp **google/brave pagination = sequential page turns** (direct
  `/search?q=..&start=..` / `offset=..` URLs, merged like `http_search`). Page
  turns are paced (~0.5-0.9s) rather than fired in parallel tabs: Google walls
  simultaneous `/search` requests from one browser, so multi-tab pagination
  returned FEWER results (measured 12 vs 30 sequential) — sequential pacing
  wins on results. Keeps the fast consent gate + trusted flow. Bounded by
  `max_pages` (≤4 pages). Verified: `--limit 30` ≈ 30 results / 13.5s on a
  clean IP.
- **core**: **brave guest-browser flow parity with google** — brave now runs
  the same multi-tab trusted flow (consent dismiss incl. OneTrust gate, humanize,
  walled-IP retry loop, `offset` pagination for limit > 10). Previously brave
  rendered a single page with no consent handling → 0 results on a consent /
  PoW-CAPTCHA wall. Note: brave PoW-captchas flagged IPs ("Verify you're not a
  bot") — same class as google `/sorry`; the retry/fallback chain handles it.
- **core**: fix **brave SERP parse** — Brave's current title link is the first
  `a[href]` in `.snippet` with the title text in a nested `.title`/`h2`/`h3`;
  the old `a.title, h2 a` matched nothing → 0 results on every brave page even
  when results rendered. Added `brave_parse_typed_results` regression test.
  `wait_for_results` also dropped its `innerText>500` shortcut (a captcha/consent
  wall has >500 chars of body text → it bailed before real results rendered) and
  the selector-poll budget is now 21s so Brave's PoW captcha has time to
  auto-resolve in the browser. Verified: `serp --engine brave --limit 5` ≈
  **5 results / 10.6s**.
- **cli, mcp**: **brave guest auto-launch** — brave now auto-launches guest
  Chrome on 9222 like google (ddg/bing stay pure HTTP, no browser). MCP
  `webrain_serp` routes google|brave through the guest-browser backend
  (auto-launch on connect failure; google sets `WEBRAIN_NO_STEALTH=1` like the
  CLI), while ddg|bing|auto stay HTTP; tool descriptions + browser-required
  guards updated.

- **cli**: `webrain launch` with no args opens Chrome in **GUEST MODE**
  (`--guest`) at google.com — a clean, ephemeral session with no profile state
  (no bookmarks/sign-in/history, nothing persisted). The **serp google
  auto-launch** (fresh + warm 9222) also uses guest mode now
  (`launch_chrome_guest`) — a fresh ephemeral session every run, consent modal
  always renders; the warm guest stays alive on 9222 for its in-memory session.
  `--stealth` is a no-op (the launch-flag stealth was removed — detectable
  fingerprint). Bare `webrain` (no args) keeps the MCP stdio default (Docker's
  `ENTRYPOINT ["webrain"]`); explicit `webrain launch <service> <profile>`
  keeps the persistent-profile CDP launch (the `webrain login` flow). All MCP
  configs/docs/Docker pass `mcp` explicitly, so they're unaffected.
- **core**: **trusted-commands-only google browser flow** — the serp google
  path no longer injects stealth JS (`WEBRAIN_NO_STEALTH=1` skips
  `stealth_js`) and runs **zero `Runtime.evaluate`**: page-JS polls (readyState,
  wall/consent state, on_results, organic count) are gone (fixed beats /
  `Page.getFrameTree` / the parse itself), the in-page 2captcha solve is
  removed, and element discovery now uses the **DOM + Accessibility domains**
  (`DOM.querySelectorAll`→`getContentQuads`, `Accessibility.getFullAXTree`→
  `backendDOMNodeId`→`getContentQuads`) while all interaction stays **trusted
  `Input.*`** (mouse moves, clicks, `mouseWheel`, per-key `dispatchKeyEvent`
  typing). New trait primitives with defaults: `element_center`, `consent_button`,
  `current_url`, `type_focused`. Guest launch remains google-only. Verified:
  `serp --engine google` returns real results over the trusted flow.
- **core**: SERP market defaulting — `engine_url` pins an **en-US market**
  when no `region` is given instead of letting the engine GeoIP the request (a
  localized IP turned `tokio rust` into Czech/Italian/Greek travel or banking
  pages). Per-engine locale params now always sent: ddg `kl`, bing
  `mkt`/`setlang`/`cc`, google/brave `hl`/`gl`/`lr`. Note: engines that
  GeoIP-lock a flagged/rotating IP still localize regardless of params — route
  through `--proxy <clean-IP>` for deterministic markets (the open-serp model).
- **core**: SERP limit respected per engine — `engine_url` takes `limit` and
  requests it (bing `count`, google `num`) instead of hard-capping at 10. Bing
  `first` is sent only past page 0 (open-serp rule: bing ignores a custom
  `count` while `first` is present). `http_search` now merges consecutive pages
  (bounded, with a no-progress stop guard) so `limit` > ~10 is honored where
  engines paginate. Note: bing/ddg cap anonymous requests at ~10 and ignore
  `count`/`first`/`s` on a GeoIP-locked IP — route `--proxy <clean-IP>` for
  larger limits and deterministic markets.
- **cli**: `webrain serp` **default = warm persistent profile + session** (the
  skill's real google-bypass path) — the auto-launched Chrome now stays alive on
  `9222` between runs (`std::mem::forget` the launch handle so `Drop::kill()`
  never runs), warming consent/session cookies into a trusted Google profile.
  `--fresh` becomes the explicit opt-out for deterministic consent every run.
- **cli**: `webrain serp --fresh` (google) — always launches a **brand-new
  profile + cookies** on a free port (never attaches a warm browser), so the
  consent modal always renders and is always dismissed before the humanized
  flow (deterministic anti-bot; a stale profile's cookies are never trusted).
- **core**: google browser path — browsemind-parity anti-bot hardening:
  - **Consent dismissed by a TRUSTED CDP click**, language-independent
    (browsemind `ConsentManager` recipe): phase 1 matches accept/reject buttons
    by a broad multilingual list (never "Sign in"/"Σύνδεση"), phase 2 falls back
    to the last button in `[role=dialog]` (Google's accept is last). Verified
    live: clicks "Tout refuser" / "Απόρριψη όλων" in any locale.
  - **Human-like behavior = browsemind's**: trusted CDP `Input.dispatchKeyEvent`
    per-key typing (NOT `Input.insertText` — a paste/IME insert with no
    keydown/keyup, flagged as non-human), trusted `Input.dispatchMouseEvent`
    moves before/during/after consent + into the search box, and randomized
    delays (`jitter()`, 40-120ms keystrokes / 1-2s reads) instead of fixed ones
    (fixed timing is a fingerprint).
  - **Plain Chrome launch** for google (no `--disable-blink-features=
    AutomationControlled` suppression flags — detectable; CDP-level masking
    covers webdriver). No hard-reload after navigate (bot pattern + doubled
    requests). Retries run in a FRESH TAB.
  - **False-success guard**: a walled `/search` (0 organic `#rso h3`) is no
    longer reported as results (the old fake "Gmail" artifact); it tries one
    direct `/search?q=` then falls back honestly.
  - **`--hold`** keeps the launched Chrome open after the search so you can
    watch it (press Enter to close).
  - **`--stealth`** is now a NO-OP — the `--disable-blink-features=
    AutomationControlled` launch flags were removed (Chrome shows an
    "unsupported command-line flag" warning banner for them, a visible
    fingerprint). All automation masking is CDP-level
    (`attach_and_init`'s stealth_js).
  - **Trusted scroll**: `CdpBackend::scroll` now dispatches a real CDP
    `Input.dispatchMouseEvent mouseWheel` (isTrusted=true) instead of JS
    `window.scrollBy` — every google human-like action is now trusted CDP input,
    no JS-driven actions remain in the flow.
  - **Auto-generated UA**: the HTTP engine headers derive `User-Agent` +
    `sec-ch-ua` from the REAL installed Chrome version (`chrome --version`,
    timeout-safe, cached) instead of a stale hardcoded `Chrome/145` — no
    forged-version fingerprint tell. The browser path already auto-generates
    its UA (no `Network.setUserAgentOverride`).
- **core, cli**: `--remote-debugging-pipe` support (`--fresh --pipe`) — launch
  google via CDP-over-stdin/stdout with NO listening debugging port (the open
  port is the automation fingerprint Google walls on `/sorry`). `connect_pipe`
  shares the WS payload-channel abstraction. NOTE: Chrome's pipe CDP is broken
  on Windows ("Remote debugging pipe file descriptors are not open") — works on
  Linux/macOS; on Windows the flow falls back to the other engines.
- **core**: `connect_default` treats an empty `CDP_URL` as unset (falls back to
  `9222`) so a warm persistent session is actually re-attached instead of the
  caller wrongly re-launching into a busy port.
- **core, cli**: optional 2captcha CAPTCHA solving (open-serp recipe) — new
  `webrain_core::captcha` module (ureq only, no new dep): extract `data-sitekey`
  + `data-s` from a Google `/sorry` wall, solve via `2captcha.com` with the same
  proxy, inject the token + `submitCallback()`. Gated by the `WEBRAIN_2CAPTCHA_KEY`
  env var; a failed solve falls through to the existing retry/fallback.
- **core**: serpapi.com Google provider — the standard `SERPAPI_API_KEY` env var
  routes google through the paid serpapi API (`/search.json`, engine=google),
  honoring `num` up to 100. **Tried first when `limit > 10`** (the free engines
  cap at ~10/page, so a high limit is best served by serpapi); a fallback for
  `limit <= 10` and in `auto`/fallback chains. Unset key / quota / 4xx degrades
  to fallback. Pure JSON parser is unit-tested.
- **core, cli, mcp**: per-request proxy for the SERP API — `webrain serp ... --proxy URL` and `webrain_serp`'s `proxy` param route traffic through an HTTP(S)/SOCKS proxy (e.g. `http://user:pass@host:port`). HTTP engines (duckduckgo/bing/google/auto) get a proxied `ureq` agent; the google browser auto-launch bakes `--proxy-server` into the launched Chrome so the humanized flow egresses through the proxy (IP rotation on walled IPs). An attached CDP engine keeps whatever proxy it was started with.
- **docs**: landing page (`docs/index.mdx`) bug fix + marketing sharpening:
  - **First-load flash fix** — the hero entrance now runs exactly once. When
    Mintlify's React shell re-renders after hydration and wipes the
    DOM-injected word spans, the animation layer silently re-wraps and restores
    the visible state instead of re-hiding and re-animating (the "page reloads
    2-3 times" flash). Scroll reveals also never re-hide nodes already in the
    viewport after a re-render.
  - **Verified proof band** — "Proven on real jobs." stats with real, auditable
    numbers: 407 unique products in ~17 tool calls (~46K tokens), 132 products
    in one parallel batch call, 3 browser engines, 0 runtime deps (one ~22 MB
    Rust binary).
  - **Game-changer copy** — hero lede now leads with the one-binary story
    ("Scrape, beat captchas, and transcribe videos, all on your machine"),
    eyebrow rationed to one separator, and the capabilities bento carries
    verified specifics (reCAPTCHA + no Python sidecar, offline whisper + local
    vision, Lightpanda ~2-4 s/page).
  - **Watch video preset** — the "Try the agent" playground gained a 4th demo
    (`webrain_watch`): bundled ffmpeg/yt-dlp/whisper → timestamped transcript
    → 12 frames + local Qwen3-VL-2B visual summary, all offline.
- **mcp**: `webrain_interact` `action` now advertises `drag` (trusted
  slider/drag CAPTCHA solving, `webrain_drag`) in the schema enum — the 0.6.2
  capability was implemented in `map_surface`/`call_tool` but never exposed to
  MCP clients, so no agent could discover it.
- **docs**: `reference/tools.mdx` table now shows the real selector enums
  (`what`/`action`/`op`/`mode`/`engine`) instead of legacy executor names
  (e.g. `extract_json`, `pdf_page`, `list_session`, which are not valid
  selector values and errored as `Unknown tool`), and gains a
  `webrain_drag` accordion.
- **core, mcp, cli**: structured SERP API — `webrain_serp` returns **typed**
  results (`{position,title,url,domain,snippet}`) instead of a raw results
  page. Engines: `duckduckgo` (default) · `bing` (plain HTML over the pooled
  no-browser HTTP agent) · `google` (JS-gated — browser path, see below) ·
  `brave` (JS SPA — renders in the
  connected CDP engine, works on Chrome/obscura/lightpanda) · `auto` (fetches
  all HTTP engines concurrently, merges + dedupes). Built-in recommended
  features: provider fallback (`fallback`), URL dedupe, pagination (`page`),
  safe search (`safe`) + region (`region`, e.g. `us-en`/`gr-el`), `request_id`
  + `ms` in the reply, retry with backoff (`retries`). New CLI:
  `webrain serp "query" [--engine …] [--limit N] [--page N] [--safe]
  [--region R] [--json] [--headless]`. MCP surface: `webrain_serp` (guide + tools
  reference updated). Reference/inspiration: the standalone `rust-serp-api`
  reference app, folded into webrain's single transport instead of a second
  HTTP server.
- **core, cli**: Google SERP over the browser path returns real results.
  `google` is JS-gated over plain HTTP (Google serves a CAPTCHA "unusual
  traffic" wall → `skipped: google`), so `webrain serp --engine google`
  auto-launches a **persistent-profile** Chrome (AppData `profiles/serp/google`,
  `--headless` for headless) when none is attached, then drives the homepage →
  hard-refresh → type → submit flow like a real user:
  - **Hard refresh first** (`Page.reload ignoreCache`, Ctrl+Shift+R) — drops
    the anti-bot state the `/sorry` wall sets, so even a fresh profile can get
    real results.
  - **Humanized event order**: navigate → wait `readyState==="complete"` →
    synthetic mousemove/down/up + hesitant scroll → consent dismiss (localized
    Reject-all/I-agree) — human events fire *after* load, never during it.
  - **Per-keystroke typing** (`type_text_delayed`, ~70 ms/char; Google flags a
    whole-string `insertText`) and an **eased/jittered mouse travel**
    (`mouse_move_human`) before the trusted click of the language-independent
    submit button.
  - **Stability wait** — `#rso h3` count flat across 3 polls, so the parse
    never captures a half-streamed 1-result page; click-until-navigation +
    dead-end guard bound the failure path (51s → ~20s).
  - In-parse href dedupe (nested `div[data-hveid]` previously collapsed the
    result set down to 1); 4× retry absorbs Google's intermittent IP CAPTCHA.

## [0.6.2] - 2026-08-12

### Added

- **docs**: landing page (now `docs/index.mdx`, served at `/`) rebuilt with a
  massive motion + sections upgrade:
  - **Try the agent** playground — a live scraper-LLM demo: preset workflows
    (Scrape prices / Auth + Turnstile / Batch interact) or a custom
    plain-language prompt; the agent loop streams (`webrain_navigate` →
    `observe` → `extract autoschema` → `extract schema`) into a JSON result
    card, powered by anime.js.
  - **You say / webrain does** — prompt→tool dispatch rows (grounded in the
    agent decision guide) linking to `/guides/agent-decision-guide`.
  - **Benchmark** section — animated counters (42 products in 1.4s, 3 engines,
    16 tools, up to 100× faster) + the extraction benchmark art.
  - Hero choreography — word-split headline reveal, staggered entrance
    timeline, typed install command (per-OS), all via
    `docs/styles/landing-anim.js` (self-healing against React rebuilds;
    additive/SEO-safe; `prefers-reduced-motion` respected).
  - Headings now render in Geist (Mintlify forces Inter on `h1`-`h6`; overridden
    scoped under `.landing`).
  - Landing moved `landing.mdx` → `index.mdx` and nav points to `index` so the
    landing serves at `/`.

- **docs**: capabilities bento ("Everything an agent needs to read the web.")
  upgraded with richer content and per-cell motion:
  - Seven asymmetric cells (span-2 + 1, 1+1+1, 1 + span-2) covering the 15
    intent-based tools, structured extraction, stealth login, challenge bypass,
    crawl-at-scale, read-anything, and local-AI engines.
  - Per-cell anime.js entrance layered inside each card: icon pop
    (`chip-ico`, scale 0.4→1 + rotate −90°→0, `easeOutBack`), then a staggered
    cascade of capability tags (`cap-tags` / `tool-chip`) and the
    `agent → navigate → observe → extract → JSON` agent-flow mini-line, then the
    crawl spiderweb SVG stroke-draws itself (path-based, dasharray/dashoffset).
  - Art + gradient cells: `core-art` dims the flow-art texture behind
    Read-anything; `core-grad` adds an animated drifting radial gradient to the
    two span-2 cells; hairline `shell` spotlight border on hover.
  - Capability tags per cell (autoschema/regex/table/jsonld/bm25, AES-256-GCM/
    TOTP/cookie transfer, cloudflare_challenge/blocked/captcha, batch/spider/
    sitemap/scan, PDF/video/downloads/JSON-LD/HTTP, whisper-cli/Qwen3-VL-2B)
    linking to `/reference/tools`.
  - Spiderweb rendered with `<path>` elements only (Mintlify's sanitizer strips
    `<circle>`/`<line>`; `<path>` survives — proven by the existing checkmarks).

- **docs**: new "One loop, every site." section — a living SVG circuit of the
  agent core loop (ask → navigate → observe → decide → extract), animated with
  anime.js:
  - Five arc segments draw themselves in sequence on scroll into view
    (`strokeDashoffset`), then the hub rings and inner ring join the draw.
  - A particle travels the closed ring continuously via `anime.path()`
    (motion-path following: `translateX`/`translateY`/`rotate`), with a
    slower counter-rotating particle on the inner ring.
  - The arriving tool node lights up as the particle passes (anime
    `onUpdate` progress → segment → node `.lit` highlight).
  - Conic `loop-radar` CSS sweep behind the circuit; reduced-motion keeps the
    circuit fully drawn and static (additive/SEO-safe, like every section).
  - All geometry is `<path>` only (sanitizer-safe); node chips are HTML
    absolutely-positioned over the SVG so labels stay crisp at any width.
- **docs**: setup-steps connector — a dashed hairline across Install →
  Connect → Ask draws on scroll and runs one glowing packet across it
  (`anime.path`), tying the three step cards into a single flow.
- **core**: trusted `drag` (slider/drag CAPTCHAs — press → move with the button
  held → release; crosses cross-origin iframes) and `eval_in_frame` (CDP
  isolated world inside a src-matched iframe — reads reCAPTCHA/hCaptcha/Turnstile
  puzzle geometry parent JS can't reach). Exposed as `webrain_drag` /
  `webrain_eval_in_frame`.
- **core**: reCAPTCHA v2/enterprise anchor fix — the checkbox sits at the
  anchor's top-left (~27,37), not the center; a `mouseMoved` now precedes every
  CDP click so modern widgets accept it as trusted.
- **core**: `webrain_vision` op=ask — screenshot the viewport or a clip region,
  ask the bundled local Qwen3-VL (or the cloud chain), return the answer; batch
  tiles in ONE request (numbered 1..N); `scale` upscales small captcha tiles the
  2B model misreads. Offline caption index (keyword top-k, no embedding model)
  added.
- **core**: warm llama-server singleton — no ~90s cold load per vision call;
  provider failover OpenRouter → OpenAI → Fireworks → Groq → local at a shared
  choke point with one retry (surfaces the API's own error body).

### Changed

- **core**: `screenshot_clip` gained a `scale` param (high-res crop pass); vision
  callers route through the shared provider-failover/retry post.
- **skill**: `skills/webrain/` restructured into a progressive-disclosure router —
  `SKILL.md` (identity, mandatory rules, routing table) + new `references/`
  (`core-rules`, `browser-selection`, `challenges`, `profiles`, `extraction`,
  `anti-patterns`), `workflows/protected-site.md`, and `evals/README.md`.
- **docs**: Mintlify nav reorganized around intent — new **Agent** group
  (`agent/protected-sites`, `agent/session-strategy`, decision guide) and new
  Concepts pages (`concepts/profiles`, `concepts/sessions`). Agent decision
  guide, browsers/challenges/runtime-flow concepts, troubleshooting, README,
  AGENTS.md, and the binary `AGENT_GUIDE` all re-centered on the
  persistent-profile + real-Chrome + session model (browser identity, profile,
  and session are execution state).
- **docs**: Obscura v0.2.0 rendering reflected everywhere — render builds
  screenshot + PDF (raster-backed) natively; no-render builds and lightpanda
  still have no paint engine; interactive Material and interactive challenges
  still need real Chrome.
- **cli**: `webrain doctor` no longer probes for a Python sidecar (removed the
  `python -c "import playwright, undetected_playwright"` check and its
  `stealth_solve` line).
- **mcp**: `webrain_guide` (AGENT_GUIDE) challenge section rewritten — native
  `webrain_session(op=login)` profile/login flow replaces the Python sidecar as
  the documented challenge fix; interactive CAPTCHAs need a human in the headed
  browser.

### Removed

- **scripts**: `scripts/stealth_solve.py`, `skills/webrain/scripts/stealth_solve.py`,
  and `skills/webrain/scripts/preflight.py` deleted — the Python stealth sidecar
  architecture is gone (no Python in the repo). Challenge handling is native
  (vault + TOTP; `wait_out_challenge` poll+reload in `login.rs`/`launch.rs`).

## [0.6.1] - 2026-08-09

### Changed

- **deps**: RustCrypto stack upgraded together (sha1 0.10→0.11, sha2 0.10→0.11,
  hmac 0.12→0.13, aes-gcm 0.10→0.11, getrandom 0.2→0.4) — they share the `digest`
  trait and must move as one. `getrandom::fill` replaces `getrandom()`.
- **deps**: lopdf 0.41→0.44; GitHub Actions bumped (checkout 7, upload-artifact 7,
  download-artifact 8, sticky-comment 3).
- **deps**: Dependabot enabled — weekly version-update PRs for cargo + GitHub
  Actions; security alerts + automated security fixes on.
- **ci**: changelog gate no longer blocks dependabot PRs (skipped when the actor
  is `dependabot[bot]`).
- **chore**: repo security hardening — branch protection on `main` (required PR +
  CI checks, linear history, no force-push), squash-only merging, auto-merge +
  delete-branch-on-merge, CodeQL scanning.

## [0.6.0] - 2026-08-08

### Added

- **docs**: tool surface synced to the **16-tool intent-based surface** — the
  Mintlify site (overview, quickstart, browsers, runtime-flow, scrape-at-scale,
  structured-extraction, troubleshooting, CLI reference) now teaches the
  consolidated `observe` / `interact` / `extract` / `crawl` / `scrape` / `pdf`
  tools, plus a new **`webrain_watch`** guide page
  (`docs/guides/watch-videos.mdx` — transcript + frames + vision fusion,
  `webrain install watch` / `install vision` bundles, STT env vars) and
  `webrain_batch` `op: eval`.
- **docs (SEO/AI discoverability)**: new **Agent Decision Guide**
  (`docs/guides/agent-decision-guide.mdx`), **MCP client guides** (`mcp/server`,
  `mcp/claude`, `mcp/cursor`, `mcp/copilot`), and honest **comparison pages**
  (vs Playwright, Browser Use, Firecrawl, Crawl4AI); canonical **16-tool**
  terminology in overview/quickstart; stronger page descriptions; `llms-full`
  enabled in `docs.json`; `.mintignore` hygiene (audit file + `arch_diagram.mmd`).
- **docs (landing page)**: custom **landing page** for the docs site
  (`docs/landing.mdx` — `mode: custom`, dark-tech taste design, asymmetric
  hero, 16-tool bento, engines, terminal + pipeline visuals as SVG in
  `docs/images/landing/`; styles in `docs/styles/global.css` scoped under
  `.landing`). Served at `/` by making it the first page in navigation — the
  hosted build ignores the `landingPage` config key because the Mintlify
  dashboard re-serializes `docs.json` and drops unknown keys.

### Changed

- **style**: cargo fmt — line-wraps long `surface_tests` assert calls and
  collapses a double blank line in `call_tool` (no logic change).
- **style**: cargo fmt on the watch batch — collapses the
  `llama_vision_endpoint` signature, wraps `whisper_thread.join()`, and expands
  the `whisper_source` if-chain (CI `cargo fmt --check` gate).
- **PixelRAG vision model migration** (`webrain_core::vision` + `webrain_vision_index`):
  the vision-embedding fallback (`Qwen3-VL-Embedding-2B` @ local vLLM:8000, GPU-only)
  is replaced by the **bundled local vision model** (`Qwen3-VL-2B` via llama-server,
  `webrain install vision`). `index_current_page` now captions the captured tiles in
  one batched chat call (`vision::describe_tiles`, reusing the watch llama-server
  spawn) and returns the page description as `vision`. Embeddings still power cosine
  retrieval; the local vision model supplies the real understanding.
- **watch resilience** (`webrain_core::video`): frame vision now falls back
  from a configured cloud provider to the bundled local Qwen3-VL when the cloud
  API errors (429/5xx) instead of failing the whole watch — the fallback was
  previously key-presence-only, so a rate limit killed vision even with the
  local model installed. Cloud whisper STT likewise tries every configured
  provider in order (Groq → OpenAI → Fireworks) on error, not just the first
  key set (`stt_providers`; `stt_provider` narrowed to a `#[cfg(test)]` helper).
- **captcha solve loop** (`webrain_solve_captcha` tool + `video::solve_captcha`):
  screenshots the exact viewport at scale 1 (so vision pixels == click pixels),
  uses the bundled local Qwen3-VL-2B to locate a reCAPTCHA checkbox, and clicks
  it via trusted CDP `click_coords`, looping until solved/puzzle/timeout.
  `--image-min-tokens 1024` added to the llama-server spawn for coordinate
  grounding. Verified live: clicked Google's `/sorry` checkbox and advanced it
  to the image puzzle (checkbox stage solved; image grids return `puzzle`).
- **offline tile vision** (`webrain_vision` + `vision::index_current_page`):
  the tile index no longer hard-fails when no embedding backend is configured
  (was `401` without an `EMBED_URL` key). It now falls back to **per-tile
  captions** from the bundled local Qwen3-VL (`VectorStore` gains a text store +
  keyword `retrieve`), so `webrain_vision` works fully offline. Captions are
  fetched in **batched groups** like `watch` batches frames (numbered per-tile
  answers, `parse_numbered`) — one llama-server spawn for all tiles.
- **captcha solver hardened + generalized** (`webrain_core::video`,
  `webrain_core::backends::cdp`, `webrain_vision`):
  - **trusted clicks register now** — `dispatch_click` sends a `mouseMoved`
    before press+release (reCAPTCHA v2/enterprise silently ignored a
    press+release with no prior pointer position — the cause of every "fake"
    click); `wait_turnstile_token` clicks the checkbox **top-left offset**
    (27,37), not the iframe center (center is the label text).
  - **empty captions root-caused** — `--image-min-tokens 1024` made every image
    ≥1024 tokens, so a 4-tile batch overflowed the `-c 4096` context and
    llama-server rejected every request; context is now `-c 16384` and vision
    errors surface instead of returning `" |  |  | "`.
  - **generic solver** — `solve_puzzle` is provider-agnostic: isolated-world
    grid discovery (any visible `img` grid, prompt from first `strong`/heading,
    verify by button text) + **one-shot classification** (screenshot the whole
    puzzle frame → 1 llama call → matching tile numbers; no tile cropping).
    Ground truth = the `*-response` token; vision "DONE" is never trusted
    without it.
  - **`webrain_interact drag`** — new trusted drag action (CDP press → move
    with button held → release) for drag-and-drop / slider CAPTCHAs; crosses
    cross-origin iframes like clicks.
  - **`webrain_vision op=ask`** — new workflow tool: screenshot a viewport/clip
    region → bundled Qwen3-VL answers an arbitrary prompt (captchas/visual QA).
    New `scale` param upscales the clip (crop+upscale precision pass for small
    captcha tiles; `screenshot_clip` gained a `scale` param, all other callers
    pass 1.0).
- **captcha-solve skill** (`skills/webrain/workflows/captcha-solve.md`): step-by-step
  algorithm any LLM follows to solve ANY captcha (reCAPTCHA/hCaptcha/Turnstile/
  2captcha xcaptcha) with webrain vision — token ground truth, checkbox claim,
  scaled `op=ask`, exact `webrain_eval_in_frame` geometry, parallel clicks,
  expiry loop, anti-patterns. Added the **TEXT/ASSEMBLE-CODE** flow: extract
  the shared data-URL sprite from the same-origin iframe (deterministic,
  expiry-immune), crop tiles, OCR each single-image (Qwen3 multi-image reads
  truncate + hallucinate), click the 2 matching tiles **in order**, Confirm →
  token.
- **cloud-first vision chain + failover** (`webrain_core::video`,
  `webrain_core::vision`): every vision path now routes through one
  `vision_targets` provider list in priority order — **OpenRouter (Qwen3.6-27B)
  → OpenAI → Fireworks → Groq → bundled local Qwen3-VL-2B**. `ask_viewport`
  captures pixels ONCE and tries each provider in order, so a flaky provider
  falls through to the next instead of killing the op (live hit: OpenRouter
  returned empty, Groq was fine). `post_vision` (the shared choke point) now
  retries ONCE on transient failures (429/5xx/network/empty output) for every
  caller — no more per-caller retry loops — and parses content as string OR
  content-block array OR `message.reasoning` (Qwen3/OpenRouter put the answer
  there when `content` is empty). One unit test pins the failover order.
- **monolithic captcha solver removed** (`webrain_core::video`,
  `webrain-mcp`): `webrain_solve_captcha` + `solve_captcha`/`solve_puzzle`/
  `captcha_token`/`BFRAME_JS`/`PUZZLE_JS` deleted. The generic tool flow
  (checkbox claim → scaled `op=ask` tiles → exact geometry → parallel clicks →
  verify → token) — live-verified to solve a reCAPTCHA grid end-to-end — is now
  the ONLY path, and its exact-geometry capability moved into a new generic
  tool **`webrain_eval_in_frame`** (run JS inside a cross-origin iframe via a
  CDP isolated world: grid tile rects + verify button for any challenge frame,
  which `webrain_eval` cannot reach).
- **watch perf** (`webrain_core::video`): frames + whisper now run in parallel;
  local vision gets **10 frames** (was 3); the bundled llama-server stays alive
  across calls on Unix (static keepalive), saving ~3s of model load per watch;
  `webrain_watch` carries per-step `ms` timing. Compiler warnings fixed.
- **install progress** (`webrain install ...`): every package download now
  prints a live `\r` progress line (`X / Y bytes (N%)` when the server sends
  Content-Length, else MiB) — no more silent "Downloading …" hang. Both
  `download_bytes` (in-memory) and `download_to_file` (to-disk, vision model)
  stream through one shared `download_stream`; also lifts the old 10 MiB
  `read_to_vec` cap. Verified live: `install watch` fetched yt-dlp with
  percentage progress and completed.
- **build(deps)**: `Cargo.lock` synced to the 0.5.0 workspace crate versions
  (`webrain-core`/`webrain-mcp`) — the v0.5.0 release commit bumped `Cargo.toml`
  but dropped the lock update.
- **tools**: compressed the MCP surface from **63 → 16 intent-based tools**
  (firecrawl-style): `navigate / observe / interact / extract / scrape /
  batch / crawl / search / pdf / download / watch / session / vision / eval /
  guide`. Every capability is preserved as a `what` / `action` / `op` / `mode`
  selector routed to the existing executor via `map_surface()`. Legacy tool
  names still dispatch (backward compatible); legacy schemas kept as
  `legacy_tool_schemas()` reference. AGENT_GUIDE + README rewritten for the
  new surface.
- **login** + **stealth**: bypass-first login — `webrain_login` keeps the
  simple poll loop (fill+submit → session cookie, or 2FA/approval →
  `waiting_for_human`), because the browser is built to **not trigger**
  challenges in the first place: CDP-direct (no chromedriver `cdc_` marker),
  `--disable-blink-features=AutomationControlled` with no automation flag,
  per-profile `user-data-dir` so `cf_clearance` persists across logins.
  `Function.prototype.toString` native-code spoof (uc pattern). NO forged
  UA / userAgentMetadata / `Emulation.setAutomationOverride` (patchright parity).
- **stealth** (patchright parity): fingerprint noise is now **ON by default**
  (`WEBRAIN_STEALTH_NOISE=0` opts out) — the canvas/audio/WebGL spoofs +
  `hardwareConcurrency`/`deviceMemory`/`connection` lies are exactly what the
  managed Cloudflare challenge measures (real values leave it stuck on "Just a
  moment…", verified vs scrapingcourse cf-antibot). New puppeteer-extra
  evasions: `window.outerdimensions`, cross-origin `iframe.contentWindow`
  proxy (HEADCHR_IFRAME), `media.codecs` canPlayType ('probably' for
  avc1/mp4). Core stealth (webdriver→false, real plugins, `window.chrome`,
  Function.toString spoof) stays always-on.
- **core**: native Cloudflare/anti-bot + captcha solving — `wait_out_challenge`
  (the Python sidecar's poll+reload loop, now in Rust) + captcha widget
  claiming (Turnstile/reCAPTCHA/hCaptcha iframe-claim + CDP center click) +
  `wait_turnstile_token` (never submit an empty token). `webrain_login` and
  `webrain open` auto-wait challenges before form-fill.
- **core**: real Chrome is preferred for launch (patchright #1 — CfT/Chromium
  is more fingerprintable); the CfT cache is the fallback only.
- **login**: `has_session` now catches site-specific `*session*` cookie names
  (PHPSESSID, laravel_session, connect.sid, …), not just the exact list.
- **scripts**: `stealth_solve.py` prefers the patched **patchright** Playwright
  driver (falls back to playwright + undetected_playwright), real-Chrome-first
  binary detection, and interactive **Turnstile** checkbox click + token wait.

### Fixed

- **tools** (`webrain_download` http + ytdlp engines): the http
  engine derived the output filename from the URL *including its query string*,
  which is an invalid Windows filename (`os error 123`) on signed CDN URLs
  (`...jpg?stp=...&sig=...`) — it now strips `?`/`#` before saving. The ytdlp
  engine resolves its binary via `install::find_tool("yt-dlp")`, so the bundled
  yt-dlp (`webrain install watch`) is used before PATH — no more
  `yt-dlp: command not found` on systems without it installed.
- **install downloads** (`webrain install vision/watch`): downloads were
  **faking completion** — `download_to_file` pre-allocated the `.part` to full
  size (`set_len`) before any bytes arrived, so a killed run left a full-size
  empty `.part` and the next run's length check "resumed" a file of zeros. Now
  **completion is tracked per-chunk**: each of 8 parallel Range chunks writes
  its index to a `<dest>.part.done` sidecar only after fully finishing, the
  file is renamed ONLY when every chunk is marked, and a `<dest>.ok` marker
  (not size) proves a dest is genuinely complete. A killed run resumes the
  missing chunks instead of faking success. Progress is now an animated
  single-line bar (`█░`, %, human MB/GB, live MiB/s) instead of scrolled raw
  bytes; `install_vision_model` re-validates every file against the server each
  run (probe Content-Range) and prints explicit `[1/2]`/`[2/2]` + status lines.
- **install (all engines)**: `download_bytes` (used by obscura, chrome,
  ffmpeg, whisper, yt-dlp, lightpanda) now routes through the same parallel
  chunked `download_to_file` — every install gets the animated progress bar +
  honest per-chunk completion, not just the vision model. Servers that ignore
  `Range` (GitHub API JSON etc.) fall back to a plain single-stream copy via
  `download_plain`, so release-metadata fetches keep working.
- **watch** (`webrain_watch`): accepts `url` as a fallback when `source` is
  absent — one less argument to get right for a single-video watch.
- **screenshot** (`webrain_core::backends::cdp`): `Page.captureScreenshot`
  now sends `captureBeyondViewport` (the real CDP param) instead of `fullPage`
  (a Playwright abstraction CDP silently ignored) — full-page screenshots on
  obscura v0.2.0's native renderer finally capture the whole page, not just
  the viewport.
- **install** (`webrain install --engine obscura`): falls back to the
  **v0.1.11** asset when `/latest` has no matching platform/stealth package,
  instead of failing the whole install on a brand-new release that's still
  uploading binaries.
- **install** (`install::find_tool`): finds the bundled yt-dlp even though it
  installs as `yt-dlp_linux` / `yt-dlp_macos` / `yt-dlp_linux_aarch64` (not
  the bare name) — fixes `watch`/`download engine:ytdlp` on Linux/macOS with
  `No such file or directory (os error 2)`.

### Changed

- **spider** (`webrain_core::engines::SpiderEngine`): added
  `with_nav_opts`/`nav_opts` — every page fetch now goes through
  `navigate_opts` so `NavOpts` (blocked resources, wait, timeout) apply to the
  crawl path too, matching the single-tool navigate path (the `NavOpts`
  `wait_timeout_secs` now also caps the Queen-Reader wait loops in `cdp.rs`,
  with `Debug` derived for the struct).
- **obscura** (`webrain_core::launch`): `launch_obscura` now passes
  `--stealth` by default (BoringSSL stealth build) so the spawned obscura CDP
  server is the anti-bot posture, not the vanilla build.
- **obscura install** (`webrain install --engine obscura`): explicit v0.2.0
  package selection — `--stealth` and `--no-render` map to the exact asset
  suffix (`-stealth` / `-no-render` / `-no-render-stealth` / plain render)
  instead of a length-sort heuristic, so the install pulls the build you ask
  for (e.g. `render+stealth` = screenshots + anti-bot, verified live).
- **docs**: obscura v0.2.0 renderer notes (README + `AGENT_DECISION_GUIDE`)
  — v0.1.11 vs v0.2.0 split in the anti-bot/screenshot table, auto asset
  selection called out.

## [0.5.0] - 2026-08-06

### Added

- **watch videos** (`webrain_core::video` + `webrain_watch` + `webrain watch`):
  transcript + frames for ANY video (URL or local file), no browser needed.
  Pipeline: yt-dlp captions → Whisper STT fallback (provider keys env-only,
  first set wins: GROQ_API_KEY → OPENAI_API_KEY → FIREWORKS_API_KEY; model
  override WEBRAIN_STT_MODEL) → ffmpeg frames (keyframe / scene-aware /
  none). `detail`: `transcript` (captions only), `efficient` (keyframe pass,
  cap 50), `balanced` (scene-aware, cap 100, default). Batch: `sources[]` →
  one parallel call, bounded worker pool, one result per video in input order.
  Single impl shared by the MCP dispatch + the lib.rs no-browser short-circuit
  (`tools::watch_from_args`), so it works on a fresh install before any browser.
  VTT parser + frame-budget unit tests.
- **watch mono bundle** (`webrain install watch`): one command downloads the
  whole self-contained `webrain watch` stack as mono packages into the webrain
  cache — ffmpeg+ffprobe (BtbN builds), yt-dlp (single binary), whisper-cli
  (whisper.cpp prebuilt), + a GGUF model. `watch` auto-resolves bundled → env →
  PATH (`install::find_tool`), so it works on any OS with no system installs
  (macOS: whisper-cli via PATH/brew, no prebuilt).
- **local whisper backend** (`webrain_core::video` + `webrain install whisper`):
  `watch` now transcribes **locally/offline** via whisper.cpp's `whisper-cli`
  when it's installed (binary on PATH or `WEBRAIN_WHISPER_BIN`, GGUF model via
  `WEBRAIN_WHISPER_MODEL`), falling back to the cloud Whisper API only when no
  local binary/model is present. `webrain install whisper [--model small.en]`
  downloads the GGUF model into the engine cache dir (reuses install.rs; model
  whitelist, no arbitrary paths). Audio extracted as 16kHz mono s16le WAV (one
  file, both backends). whisper-cli JSON parser unit-tested.
- **watch vision fallback** (`webrain_watch vision:true` / `webrain watch --vision`):
  after frames are extracted, up to 3 evenly-sampled frames are sent to a vision
  LLM (Groq `qwen/qwen3.6-27b` → OpenAI `gpt-4o-mini` → **local Qwen3-VL-2B**
  via bundled llama-server when no key is set; cloud env keys same as STT)
  and the response is returned as `vision` — text captions + a fused
  visual summary. Use when the client can't render frame images (text-only
  model / MCP client). DeepSeek's hosted API is text-only — live-verified
  2026-08-06 (`deepseek-v4-flash`/`deepseek-v4-pro` reject `image_url` content:
  `unknown variant 'image_url', expected 'text'`), so it's excluded from the
  chain despite a blog claiming vision support. Opt-in (API cost); failure
  surfaces as `vision_error`, never fails the watch.
- **local vision hero** (`webrain install vision` + `webrain_watch vision:true`
  with NO key): when no GROQ/OPENAI key is set, `watch --vision` falls back to
  a **local Qwen3-VL-2B** served by a bundled llama.cpp `llama-server` — the
  whisper analog, cache-contained (no system installs, all OS). `webrain
  install vision` downloads llama-server (CPU build, ~10-18MB, always the
  latest release via the GitHub API) + the unsloth `Qwen3-VL-2B-Instruct-
  Q4_K_M.gguf` + `mmproj-F16.gguf` (streamed to disk, ~1.8 GiB) into the
  cache; `install watch` bundles the stack too. `watch` auto-resolves it
  (`install::vision_local`), spawns llama-server on a free port, POSTs the 3
  frames to its OpenAI-compatible `/v1/chat/completions`, and kills it after.
  Model name comes from the gguf basename. Verified chain selection via unit
  test; the 1.8 GiB model download + live local run is opt-in (run `webrain
  install vision` once).
- **install warnings** (`webrain install watch`): returns a `warnings` block
  when the watch stack is unusable after install — no local whisper (binary +
  model) AND no cloud STT key (GROQ/OPENAI/FIREWORKS) → caption-less videos
  yield an empty transcript; no vision key (GROQ/OPENAI) → `watch --vision`
  returns `vision_error`. One chokepoint covers the whole watch tool.
- **batch eval** (`webrain_core::engines::batch_eval` + `webrain_batch`): new
  `op=eval` — run arbitrary JS in every tab, return the JSON per URL. The
  "custom extractor" op for hashed/SPA DOMs; no CSS schema needed. This is what
  the 20-post Instagram extraction should have used.
- **cookies --netscape** (`webrain-cli`): `webrain cookies --out f --netscape`
  writes Netscape HTTP Cookie File — the format yt-dlp/curl `--cookie` want
  (yt-dlp rejects the JSON shape).
- **doctor hints** (`webrain-cli`): `webrain doctor` now (a) prints the start
  command when the MCP server (9223) is down, and (b) reads `/json/version` per
  CDP port to flag **relay/tunnel** (websocket URL points at a different port —
  the wslrelay/Docker case that looked like local Chrome) and **headless**
  (`HeadlessChrome` in UA — cannot pass login challenges).
- **screenshot path** (`webrain_screenshot`): still returns `screenshot_b64`,
  but now ALSO writes the PNG to `dir` (default `screenshots/`) and returns
  `path` so any client can view the file instead of decoding base64.

### Changed

- **style**: cargo fmt on the agent-browser batch (CI `cargo fmt --check` gate).
- **media guidance** (`AGENT_GUIDE` + `webrain_extract_json`): extract media
  URLs from `meta[property='og:image'|'og:video']` — not `video.src`, which is a
  blob: URL for streamed media (reels/DASH) and can't be downloaded.
- **config**: smart commit scope list — new `commitlint.config.js` (canonical),
  enforced on PR titles by `pr-lint.yml`, and mirrored in CONTRIBUTING.md /
  contributing.mdx / README / copilot-instructions. Scopes now reflect the
  real crates + subsystems (drops the borrowed plugin/handlers/pipeline/serp
  scopes).

### Fixed

- **login truth** (`webrain-core/src/login.rs` + `webrain-mcp`): `webrain_login`
  stopped lying. `SESSION_COOKIES` no longer includes `datr`/`dpr`/`mid`/`ig_did`
  — those are set on the login page while logged OUT, so `has_session()` reported
  `logged_in:true` falsely; added `ds_user_id`. `run_login` now detects a
  reCAPTCHA/anti-bot challenge (`challenge:"captcha"` + `waiting_for_human:true`
  instead of a misleading "no session cookie — check creds") and early-outs on a
  tablet/app interstitial when no login form is found. `webrain_login` MCP
  dispatch wired to the native `login::run_login` (matching the schema + CLI)
  instead of the stale selector path that always errored. Regression test:
  `login::tests::session_cookies_exclude_login_page_only`.
- **download collision** (`engines::download_ytdlp`): output template now
  `%(title)s_%(id)s.%(ext)s` — two posts from the same uploader share yt-dlp's
  `Video by <user>` title and silently overwrote each other; the media id makes
  names unique.
- **antibot false positive** (`browser::detect_antibot`): dropped the bare
  `("captcha", "captcha")` marker — it flagged any page whose text merely
  mentions "captcha" (every Instagram page). Kept the real widget markers
  `h-captcha`/`g-recaptcha`.

## [0.4.0] - 2026-08-06

### Added

- **tools** (`webrain_add_init_script`, agent-browser `--init-script`/
  `addinitscript` borrow): register a JS init script that runs via
  `Page.addScriptToEvaluateOnNewDocument` before EVERY future navigation (new
  documents only — already-loaded pages aren't rewritten). `CdpBackend` keeps a
  shared script list replayed in `attach_and_init` for every new tab, plus a
  live register on the active session. Unblocks the `webrain_flatten` ceiling:
  closed-shadow-root piercing via an `attachShadow` patch injected as an init
  script. `ponytail:` no remove tool — scripts accumulate per session; add
  `webrain_remove_init_script` when a real case needs it. Verified live: a
  `window.__WBR` stub did NOT appear on the loaded page, but DID on the fresh
  document after the next navigate.
- **signal** (`PageState.chrome_error`, spider-rs `is_chrome_error_page`/
  `extract_chrome_error_code` borrow): Chrome renders dead-domain/cert/5xx URLs
  as an error page that LOOKS like content — the LLM scraped garbage silently.
  New `detect_chrome_error(title,text)` returns the `ERR_*`/`DNS_*` code (e.g.
  `DNS_PROBE_FINISHED_NXDOMAIN`) or `CHROME_ERROR` for a generic interstitial.
  Computed in both `PageState` builders (navigate + snapshot), surfaced on
  `webrain_navigate`/`webrain_snapshot`/`webrain_search`. Verified live: dead
  domain → `DNS_PROBE_FINISHED_NXDOMAIN`, example.com → null.
- **signal** (`webrain_fetch_http` → `needs_js` + `visible_chars`, spider-rs
  `smart` mode lazy slice): the no-browser HTTP probe now reports whether the
  raw HTML is a JS shell (`needs_js: true` when it says JS is required, or the
  visible text is <100 chars of an HTML shell) so the LLM can upgrade to the
  browser instead of scraping an empty page. Wired into the live no-browser
  dispatch in lib.rs (the tools.rs arm is dead code for this tool). Verified:
  example.com `needs_js:false` (285 visible chars), notion.so `needs_js:true`
  (45 visible chars).
- **tools** (`webrain_select`/`webrain_hover`/`webrain_check`/`webrain_dialog`/
  `webrain_wait`/`webrain_upload`, agent-browser borrows): 6 form/interaction
  primitives on the ELEMENTS_JS index model.
  - `webrain_select {index, value}` — native `<select>` option by value OR
    visible text, fires a real `change`; **no-match errors listing available
    options** so the LLM self-corrects.
  - `webrain_hover {index}` — trusted `Input.dispatchMouseEvent mouseMoved`
    (JS mouseover fallback) — triggers :hover menus / hover-reveal content.
  - `webrain_check {index, checked?}` — click + verify via agent-browser
    `is_element_checked` (native .checked → aria-checked → label.control →
    nested input); JS label-retarget fallback; returns ACTUAL state.
  - `webrain_dialog {action, prompt_text?}` — `Page.handleJavaScriptDialog`;
    unblocks a session paused by a sync `alert()`/`confirm()`/`prompt()`.
  - `webrain_wait {ms | selector | text, timeout_ms?}` — standalone post-action
    wait (click→AJAX→render); navigate already waits internally.
  - `webrain_upload {index, files[]}` — `DOM.setFileInputFiles` via resolved
    node id.
  All verified live on real Chrome (scripts/test_agentbrowser_steals.ps1):
  select sets value + change, no-match lists options; check toggles + reports;
  upload lands the file; dialog resolves a paused alert and the page continues;
  wait satisfied for selector/text and false for absent.
- **fix** (click-hang on sync dialog): `dispatch_click` now bounds the CDP
  input ack wait (agent-browser `dispatch_mouse_or_dialog` borrow). A sync
  `alert()` in a click handler pauses the renderer so the ack never arrives —
  previously `webrain_click` (and every later eval) hung forever. On timeout
  the click is treated as dispatched and `webrain_dialog` resolves the dialog.
  Engine errors (no `Input.*`) still propagate → JS fallback preserved.
- **tools** (`webrain_flatten`): full
  composed page text including **Shadow DOM**. Web-Component sites
  (Lit/Stencil/Shoelace) render content in shadow roots invisible to
  `querySelectorAll`/`innerText` — this walks light DOM + open shadow roots
  (recursing nested roots, resolving `<slot>` projections) and returns the
  dense composed text (`{status, chars, words, text}`). Verified live:
  `document.body.innerText` returned `""` on a shadow-DOM test page while
  `webrain_flatten` returned the shadow content. `ponytail:` open roots only —
  closed roots need an `attachShadow` patch injected before component creation.
- **tools** (`webrain_annotate`):
  numbered red boxes overlaid on interactive elements at viewport coords +
  a legend `[{n, index, tag, text}]` where `index` maps directly to
  `webrain_click`/`webrain_type` indices. Returns `{status, screenshot_b64,
  legend}` and removes the overlay after capture. Built for vision models —
  read the labels, then click by index. Verified live on example.com: legend
  `[{"index":0,"n":1,"tag":"A","text":"Learn more"}]`, red pixels confirmed
  in the PNG, overlay removed afterward. Viewport screenshot only.
- **tools** (`webrain_fit`): no-query
  dense-content extractor. Walks the DOM, scores leaf blocks (`P`, `H*`, `LI`,
  `TD`, `BLOCKQUOTE`, `PRE`) by text-vs-link density + tag importance, prunes
  nav/footer/aside/form/header chrome, and returns the "fit" text — the meat of
  the page for the LLM instead of raw `innerText`. Containers (`MAIN`/`ARTICLE`/
  `SECTION`) always descend so a page wrapper can't dump everything (Wikipedia
  `<main>` case). Verified live: pruned the Wikipedia toolbar + Categories,
  returned the dense article.
- **tools** (`cdp_session_*`): `click`,
  `type_text` and `press` now use trusted CDP `Input.*` events
  (`Input.dispatchMouseEvent` / `Input.insertText` / `Input.dispatchKeyEvent`)
  with a JS fallback for engines without `Input.*` support (lightpanda).
  New `webrain_click_coords` tool — trusted click at raw viewport coords for
  cross-origin iframe content / reCAPTCHA. Verified live on real Chrome
  (click fires handlers, Tab moves focus) and against the docker obscura
  engine (runs clean, no regression).
- **fix** (double-fire): `dispatch_click` no longer appends a JS
  pointer/click dispatch after the CDP `Input.dispatchMouseEvent` — CDP already
  fires the click, so the JS fallback double-fired handlers on every engine.
  `webrain_click` / `webrain_click_coords` now fire exactly once (verified via a
  click counter on both real Chrome and docker obscura).
- **tools** (backend-node click, browsemind `cdp_session_click_backend`):
  `webrain_click` now prefers a stable backend-node click —
  `DOM.getDocument` → `DOM.querySelector` (webrain's own per-element selector)
  → `DOM.getContentQuads` → trusted `Input.dispatchMouseEvent` at the quad
  center. `backend_node_id` survives incremental DOM mutations that stale
  viewport coords don't. Chrome/Chromium (incl. Playwright chromium on Linux)
  engage it; obscura/lightpanda (no layout/quads) fall back to the viewport-
  coord path then element-based `el.click()`. Verified: real quads returned on
  Chrome (`getContentQuads` probe), single-fire on both real Chrome and docker
  obscura.
- **tools**: crippled-page detection — `webrain_navigate` / `webrain_snapshot`
  return a `crippled` field (loaded page with <5 interactive elements and no
  challenge → likely a bot-limited shell, e.g. YouTube/Twitter/X stripped
  pages). Soft hint, not a block.
- **docs**: obscura interaction notes — obscura implements
  `Input.dispatchMouseEvent`/`dispatchKeyEvent` but NOT `Input.insertText`
  (falls back to JS fill), has no layout engine so coordinate clicks rely on
  its internal `elementFromPoint`, and does NOT parse inline `onclick=`
  attributes (use addEventListener). Lightpanda lacks `Input.*` entirely
  (JS fallback).

### Docs

- **multi-agent delegation doctrine** (enriched from browsemind
  EXTRACTION_GUIDE + DYNAMIC_DATA_EXTRACTION_GUIDE): the in-binary
  `AGENT_GUIDE` (returned by `webrain_guide`) and
  `docs/AGENT_DECISION_GUIDE.md` now teach the LLM a first-class delegation
  pattern — when to spawn parallel subagents, the subagent contract (one
  browser/CDP_URL, one task, compact JSON only, report challenges), and how the
  orchestrator aggregates (dedupe sliding windows, BM25, count). New §4c in the
  doc + delegation rows in the extraction matrix. Second pass adds the browsemind
  patterns: **delegation as the LAST parallel lever** (in-browser
  `webrain_batch concurrency`/`cdp_urls` first — don't spawn subagents for what
  one batch handles), a **delegate-by-pattern table** (catalog/specific-pages/
  infinite-scroll/whole-site/discovery-vs-extraction → default vs delegate vs
  shard), **subagent self-heal fallback chains** (extraction fit→flatten→
  extract_json→eval→annotate; pagination construct→click→scroll→scan; anti-bot
  stop+report), the **pagination type decision tree** (§4: A numbered links →
  validate→batch, B Next-only → click loop, C Load More, D infinite scroll, E
  /page/N → construct+batch, F unknown → spider), and an **SPA hydration wait**
  tip (poll DOM growth with webrain_wait before extracting a JS shell).

### Fixed

- **cli**: `webrain upgrade` on Windows/Scoop now closes other running webrain
  instances before `scoop update` — the MCP server kept the exe locked, so
  Scoop refused to replace the binary and the upgrade never landed. It now
  stops the siblings (self exits right after spawning scoop) and the update
  completes.

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
