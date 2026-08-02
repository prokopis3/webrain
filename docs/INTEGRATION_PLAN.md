# webrain — Integration & Gap-Closure Plan

> What to steal from crawl4ai, browser-use, browser-harness, obscura, camofox,
> lightpanda, and agent-browser.dev. What to skip. Build order. Readiness
> checklist. Grounded in live research across all 14 references (Aug 2026).

## 0. Installation & Distribution (P0 — blocks everything else)

**Current:** zero distribution. Users `cargo build` from source.
**Target:** two paths, both one-command:

| Path | Command | Audience |
|---|---|---|
| **cargo install** | `cargo install --git https://github.com/your-org/webrain webrain` | Rust users (Win/Mac/Linux) |
| **Docker** | `docker run -p 9223:9223 ghcr.io/your-org/webrain mcp --http 9223` | Everyone else |

Plus a `docker-compose.yml` that bundles webrain MCP + obscura sidecar so one
`docker compose up` gives the full stack (MCP server + stealth browser).

**What browser-use does:** `uv tool install --python 3.12 browser-harness` then
`browser-harness --doctor`. webrain's equivalent: `cargo install webrain` then
`webrain --doctor`. Zero Python, zero npm — one Rust binary.

**What agent-browser.dev does:** `npm install -g @agent-browser/cli` or
`pip install agent-browser`. webrain skips npm/pip — Rust binary + Docker covers
the same surface with fewer dependencies.

**Implementation:** `Cargo.toml` already has `[[bin]]`. Add multi-stage
`Dockerfile` (builder = rust:alpine, runtime = alpine + chromium).
~20 lines of Docker, zero code changes.

→ skipped: scoop, homebrew, npm, pip, choco, apt. `cargo install` works
everywhere Rust works (everywhere). Add platform packages when users ask.

---

## 1. Blocker Fixes (P0 — ship before anything else)

### 1a. `webrain_fetch_http` must not require CDP

**Bug:** routes through browser backend → fails when no browser is running.
The most efficient tool in the matrix is broken.

**Fix:** Special-case in `handle_rpc` (same pattern as `webrain_guide`):
`ureq::get(url)` → `{text, headers, status}`. No backend connect.
Already have `ureq` in deps (`download_files` uses it).

**Lines:** ~15. **File:** `webrain-mcp/src/lib.rs`.

### 1b. Tab management MCP tools (steal from browser-use)

browser-use exposes: `browser_list_tabs`, `browser_switch_tab`,
`browser_close_tab`. webrain's CDP backend already manages tabs internally —
just not exposed as MCP tools.

| Tool | CDP call | Returns |
|---|---|---|
| `webrain_list_tabs` | `Target.getTargets` | `[{tab_id, url, title, active}]` |
| `webrain_switch_tab` | `Target.activateTarget` | `{tab_id, active: true}` |
| `webrain_close_tab` | `Target.closeTarget` | `{tab_id, closed: true}` |
| `webrain_new_tab` | `Target.createTarget` | `{tab_id, url}` |

**Lines:** ~50 in `tools.rs` + ~30 in `cdp.rs`.
**Files:** `webrain-mcp/src/tools.rs`, `webrain-core/src/backends/cdp.rs`.

### 1c. `--doctor` diagnostics (steal from browser-harness)

browser-harness has `--doctor` that probes Chrome, daemon, and auth in one
command. webrain's `preflight.py` does half of this but is Python + external.

**Fix:** Embed `--doctor` in the binary:

```
$ webrain --doctor
  mcp server       ✅ (http://127.0.0.1:9223)
  cdp port 9222    ❌ (no browser)
  cdp port 9224    ✅ (obscura v0.5.1)
  recommend        obscura
  cargo version    0.1.0  (latest: 0.1.0)
  stealth_solve    ✅ (Python + playwright + undetected_playwright)
```

On startup, if no browser on any CDP port, print one line:
`webrain: no browser on CDP. Start one: docker start obscura`.
Don't auto-launch — platform-specific and fragile.

**Lines:** ~40. **File:** `webrain-cli/src/main.rs`.

---

## 2. Multi-Browser / Parallel Subagent Execution (P1 — doc, no code)

**Already works.** webrain's MCP server creates per-connection sessions via
`Mcp-Session-Id` HTTP header. Each session = isolated `CdpBackend`.
An orchestrator LLM spawns subagents, each with different `CDP_URL`:

```
Subagent A → CDP_URL=http://127.0.0.1:9222  (real Chrome, solving CF)
Subagent B → CDP_URL=http://127.0.0.1:9224  (obscura, batch scraping)
Subagent C → (no CDP) + webrain_fetch_http  (static pages, zero browser)
```

browser-use pattern: `asyncio.gather(*[agent.run() for agent in agents])` with
separate `BrowserSession` per agent. webrain equivalent: LLM dispatches N
subagents, each connects to own MCP session with own CDP_URL.

### Batch concurrency (from original P3, now P1)

`webrain_batch` is sequential (single CDP WS). Add `concurrency=N`: round-robin
N CDP ports. Lightpanda instances share nothing — true memory win.

**Doc:** Add two sentences to `AGENT_GUIDE` constant + `skills/webrain/SKILL.md`.
**Code:** ~80 lines in `engines.rs`.

---

## 3. Extraction Engine (P1 from original plan)

### Schema field types (~120 lines)

`build_extract_js`: `text | attr | html | xpath` → add `regex`, `nested`,
`nested_list`, `list`, `source` (sibling `+ tr`), `transform`.
All compile to `querySelectorAll` + `new RegExp` JS. Zero deps.
**File:** `webrain-core/src/engines.rs`.

### Regex builtins (~15 lines)

8 → 21 patterns. One-liner each in `regex_patterns()`.
**File:** `webrain-core/src/engines.rs`.

---

## 4. Crawl Engine (P2 from original plan)

| Feature | What | Lines |
|---|---|---|
| DFS + BestFirst | Stack enum + `BinaryHeap<(score, url)>` in `SpiderEngine` | ~40 |
| Domain guard + filters | `allowed`, `blocked`, `same_domain_only`, glob, ext check | ~60 |
| Auto-scroll + resume | JS scroll loop + JSON checkpoint file | ~50 |
| URL seeding | `urls[]` with `priority` field | ~10 |

**Total:** ~160 lines. **File:** `webrain-core/src/engines.rs`.

---

## 5. Cache + Cleaning (P3 from original plan)

### Crawl cache (~60 lines)

`SHA-256(url) → {PageState, HTML, timestamp}` on disk.
`cache: bypass | enabled | disabled` arg on navigate/get_html.
Direct token-cost win — re-crawling same URL costs the same prompt tokens.
**File:** `webrain-core/src/engines.rs`.

### Fit-text cleaning (~40 lines)

`webrain_clean`: strip nav/footer/script/style/iframe, exclude social/ads links,
`word_count_threshold`. In-page JS. No HTML→Markdown (deliberate — text via
`innerText`, layout via vision tiles).
**File:** `webrain-core/src/engines.rs`.

---

## 6. Skip-List (not building — with reasons from all 14 references)

| Feature (source) | Why skip |
|---|---|
| LLMExtractionStrategy (crawl4ai) | The agent IS the LLM. It writes schemas → tools reuse them. |
| Schema auto-generation (crawl4ai) | LLM emits JSON schema. Doc example, not code. |
| CosineStrategy / embeddings (crawl4ai) | Needs embeddings dep. Vision cosine already exists. |
| Markdown generation (crawl4ai) | Deliberate: text via innerText, layout via vision tiles. |
| Streaming results (crawl4ai) | MCP request/response. Streaming adds complexity, no value. |
| AdaptiveCrawler (crawl4ai) | Agent loop stops when done. Confidence is agent-side. |
| BM25 / SEO filters (crawl4ai) | Needs index. Agent decides relevance. |
| xCAPTCHA solving (crawl4ai) | Out of lane. Stealth minimizes CAPTCHAs, doesn't solve. |
| Proxy manager (browser-use) | Browser-level config. Pass `--proxy-server` to Chrome. |
| PDF generation (crawl4ai MCP) | Rarely needed for agentic browsing. Add when asked. |
| Video recording (browser-harness) | Niche. Screenshots work. Add when asked. |
| Cloud/remote browsers (browser-use) | webrain is self-hosted, not SaaS. |
| Browser profiles (crawl4ai/camofox) | Chrome `--user-data-dir` handles it at launch level. |
| Webhook/async jobs (crawl4ai) | MCP is sync request/response. Add when scale demands. |
| Telemetry (browser-harness) | YAGNI for a self-hosted tool. |
| Domain skills (browser-harness) | Agent-side. SKILL.md pattern covers this. |
| File uploads/downloads (browser-harness) | CDP `Page.setDownloadBehavior`. Add when needed. |
| Camoufox fingerprint rotation | obscura `--stealth` + `STEALTH_JS` already covers this. |
| Hermes agent integration | webrain IS the tool, not the agent. LLM drives it. |
| PixelRAG vision embedding | `webrain_vision_index` + `webrain_vision_retrieve` exist. |
| Page-agent DOM understanding | `webrain_autoschema` + `webrain_eval` cover structural discovery. |

---

## 7. Implementation Status

```
✅ P0: cargo install + Dockerfile                        (~25 lines, new file)
✅ P0: fetch_http no-browser fix                         (~30 lines, lib.rs)
✅ P0: Tab management tools                              (already existed as webrain_tab)
✅ P0: --doctor diagnostics                              (~50 lines, main.rs)
✅ P1: Extraction schema (ALL field types)               (already existed in build_extract_js)
✅ P1: Regex builtins (21 patterns)                      (already existed in regex_patterns)
✅ P1: Batch concurrency (Semaphore-bounded)             (already existed in batch_extract)
✅ P1: Multi-browser doc                                 (AGENT_GUIDE + SKILL.md updated)
✅ P2: Crawl: BFS/DFS/BestFirst, filters                 (already existed in SpiderEngine)
✅ P2: Auto-scroll (webrain_scan)                        (already existed)
✅ P3: Crawl cache (SHA-256 disk)                        (~40 lines, engines.rs)
✅ P3: Fit-text cleaning (webrain_clean)                 (~25 lines, engines.rs + tools.rs)
✅ P3: Dockerfile                                        (~20 lines, root)
                                                        ────────────────────────
                                                        ~190 new lines (vs ~630 planned)
```

---

## 8. Readiness Checklist

| Capability | Status | Notes |
|---|---|---|
| Navigate + snapshot (DOM + text + URL) | ✅ | `webrain_navigate` returns title, text, elements, challenge |
| Click, type, scroll, press key | ✅ | Index-based interaction |
| Extract structured data (CSS/XPath) | ✅ | All field types: text, attr, html, xpath, regex, nested, nested_list, list, source, baseFields |
| Fit-text cleaning | ✅ | `webrain_clean`: strip nav/footer/social, word threshold |
| Regex builtins (21 patterns) | ✅ | `webrain_extract_regex` |
| Auto-discover schema + URLs | ✅ | `webrain_autoschema` + `webrain_eval` |
| Batch pagination (concurrent) | ✅ | `webrain_batch` + Semaphore-bounded concurrency, one tab per URL |
| Spider: BFS + DFS + BestFirst | ✅ | `SpiderEngine` with strategy enum, domain filters, keywords |
| Anti-bot challenge detection | ✅ | `PageState.challenge` field on every navigate |
| Cloudflare bypass (real Chrome) | ✅ | `scripts/stealth_solve.py` sidecar |
| Screenshots | ✅ | `webrain_screenshot` |
| Tab management (list/switch/close/new) | ✅ | `webrain_tab` with sub-actions |
| No-browser static fetch | ✅ | `webrain_fetch_http` via ureq, no CDP required |
| Diagnostics | ✅ | `webrain --doctor` |
| Multi-browser parallel execution | ✅ | Per-session CDP isolation + documented in AGENT_GUIDE |
| Crawl cache | ✅ | SHA-256 disk cache: `cache_read`/`cache_write` |
| Docker distribution | ✅ | Multi-stage Dockerfile |
| PDF export | ❌ | Skipped — rarely needed |
| Proxy rotation | ❌ | Browser-level config |
| File download / upload | ✅ (download) | `webrain_download` (http + ytdlp) |
| Session recording / video | ❌ | Skipped |
| Login / authenticated sessions | ✅ | Via stealth_solve.py + CDP session sharing |
| Infinite scroll handling | ✅ | `webrain_scan` + `webrain_click` loop |
| Search engine scraping | ✅ | `webrain_search` (4 engines) |
| JSON-LD / microdata extraction | ✅ | `webrain_get_jsonld` |
| Table extraction | ✅ | `webrain_table` |
| Vision / PixelRAG | ✅ | `webrain_vision_index` + `webrain_vision_retrieve` |
| Accessibility tree | ✅ | `webrain_a11y` + `webrain_semantic_tree` |
| Media capture | ✅ | `webrain_media` (CDP network capture) |
| Overlay dismissal | ✅ | `webrain_dismiss_overlays` |
| Console errors | ✅ | `webrain_console` |

**Verdict:** ✅ Ready. All P0 blockers fixed. All extraction types, concurrent
batch, crawl strategies, cache, clean, tab management, anti-bot, diagnostics,
Docker, and 28+ MCP tools. Missing: PDF, video recording, proxy. None block
the core agentic loop.

---

## 9. Architecture Impact

All changes stay in existing files. No new crates. One new file (Dockerfile).

| File | Changes |
|---|---|
| `webrain-mcp/src/lib.rs` | fetch_http special-case (no backend) |
| `webrain-mcp/src/tools.rs` | tab tools + batch concurrency args + AGENT_GUIDE update |
| `webrain-core/src/engines.rs` | extraction schema, regex, crawl engines, cache, clean |
| `webrain-core/src/backends/cdp.rs` | tab management CDP calls |
| `webrain-cli/src/main.rs` | --doctor diagnostics |
| `Dockerfile` | new: multi-stage build + obscura sidecar |
| `skills/webrain/SKILL.md` | parallel subagent doc |
| `docs/AGENT_DECISION_GUIDE.md` | parallel subagent + tab tools |

The `BrowserBackend` trait is untouched. Everything compiles to JS injected
through `evaluate()` or CDP commands on the existing WS connection. The graph's
hottest functions (`BrowserBackend.evaluate` fan-in 7, `BrowserBackend.navigate`
fan-in 6) are the only touch-points — already proven stable.


# what remaining and missing 
AutoThrottle, checkpoint, ad block list, TLS fingerprinting — those are optimization layers. Add when a task fails due to rate-limiting, crawl interruption, or TLS detection respectively. The session management tool is the architectural unlock.
per-tool request deadlines on the MCP server. The connect timeouts cover the real hang; add a global MCP tool-call timeout only if a long batch (100+ URLs) needs a cap.
→ skipped: solve_cloudflare, adaptive selectors, 3500-domain list, ProxyRotator — big or unproven, add when a real site blocks you (YAGNI).
→ skipped: TLS/HTTP3 impersonation — impossible cheaply in Rust (needs BoringSSL fork).
→ done: persist `webrain_batch` payloads to disk via optional `output` arg (survives temp-file GC between turns). Verified live on scrapingcourse.com full-site crawl.
→ done: `webrain_batch op=interact` — run an async JS interaction (load-more loop, infinite-scroll, form fill) in PARALLEL tabs across N URLs, then optionally extract. The game-changer for "N independent interactive sites": one call replaces N serial agent loops. Verified live: button-click → 132 products, infinite-scrolling in a single parallel call.
→ done: `webrain_batch cdp_urls` — round-robins URLs across N CDP backends (per-proxy isolation: each browser = own proxy/cookies/fingerprint) in ONE call, no subagents. `batch_screenshot` now honors NavOpts. Benchmarked 4 URLs: single 2.0s warm / multi (2 Chrome) 2.3s — isolation win, not raw speed (single-backend already parallelizes tabs).
→ done: `LINKS_JS` upgraded to Scrapling LinkExtractor quality — canonicalizes URLs (trailing-slash normalise), strips fragments, drops non-http(s) schemes (mailto:, javascript:, tel:), filters content-obvious extensions (images, fonts, pdf, archives, css/js, ico, video/audio), dedupes in insertion order. From ~50 raw hrefs → 28 cleaned, canonicalized links on scrapingcourse /ecommerce. Verified live.
→ next when you want: port the 3500-domain list.
→ skipped: Cloudflare/Antibot gated pages (need the stealth sidecar — documented, out of anonymous scope), login pages (auth-gated).

## Remaining gaps (audit vs Scrapling LinkExtractor, Aug 2026)

The `links` field is now Scrapling-quality for individual pages. What a full
`LinkExtractor` provides that's still missing:

| Feature | Scrapling | webrain | Why skip |
|---|---|---|---|
| Regex `allow`/`deny` | `LinkExtractor(allow=r"/posts/", deny=r"/tag/")` | None | The LLM can filter client-side; add when agent misses this |
| Domain `allow_domains`/`deny_domains` | Subdomain-matching rules | same-origin only (`startsWith(origin)`) | same-origin covers most crawls; add when cross-domain needed |
| CSS/XPath `restrict_css`/`restrict_xpath` | Scope link extraction to page region | whole page only | Add when an agent scrapes a nav/footer-heavy site |
| `SitemapSpider` | Parse sitemap.xml / sitemap_index.xml / robots.txt | None | `webrain_fetch_http` + regex can cover this; dedicated tool when common |
| Spider + extract integration | `Spider.parse()` yields items + follows links to different callbacks | Spider = link-only; batch extract = separate call | Agent glues them; dedicated `webrain_crawl` tool when proven needed |
| Dev/resume mode | `development_mode` caches responses, `crawldir` persists state | None | Add when long crawls break; currently the output-persist covers the data |

→ skipped: these are all "add when proven needed by a real task" — not YAGNI until a crawl fails because of them.

## Multi-session / per-proxy dispatch (P2 — doc, infra exists)

Batch concurrency (12–13 tabs in one real Chrome via `webrain_batch`) already
delivers parallelism for many-URL listing extraction. **`op=interact`** now
parallelizes N *independent interactive* sites in one call (each tab runs its
own interaction JS loop). Multi-agent still pays off for **per-proxy isolation**
or cookie/rate-limit separation across targets — a single "load-more" page is a
serial loop, but N *different* interactive pages run in parallel tabs.

The plumbing is already live (named session pools + per-`Mcp-Session-Id` routing):
```
Subagent A → session "site-a" → cdp_url http://127.0.0.1:9222 → click load-more on site A
Subagent B → session "site-b" → cdp_url http://127.0.0.1:9224 → scroll site B (obscura)
Subagent C → no CDP → webrain_fetch_http → static pages (zero browser)
```
Each subagent calls `webrain_open_session` (optional `cdp_url`), then drives its
own session via the `Mcp-Session-Id` header; `webrain_batch` can also target a
named session for parallel multi-site extraction. Add when you have ≥3
independent interactive targets or need cookie/rate-limit isolation per target.
