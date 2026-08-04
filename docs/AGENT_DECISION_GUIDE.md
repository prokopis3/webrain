# Webrain — Agent Decision Guide (LLM reads this before using MCP tools)

How to choose the optimal path for every scraping task: **which browser**, **how
to handle a challenge**, and **which extraction tool**. Built from the
browsemind extraction guides + verified live on scrapingcourse.com.

---

## 1. Browser selection (set `CDP_URL` / pick the backend)

| Situation | Browser | Why |
|---|---|---|
| **Cloudflare / Turnstile / CAPTCHA / any interactive challenge**, or need screenshots / pixel rendering | **Real Chrome + stealth sidecar** (`scripts/stealth_solve.py`) | Only a real rendering engine can run/solve these. obscura has **no paint engine** and its V8 crashes on challenge JS. |
| JS-rendered pages **without** a challenge, high-concurrency batch scraping | **obscura** (docker, `--stealth`) | Fast, light, no Chrome overhead. Multi-tab: `webrain_batch` runs parallel tabs. Cannot do interactive challenges or screenshots. |
| Lightweight / minimal footprint static-ish JS pages | **lightpanda** (`docker run -d --rm --name lightpanda -p 9225:9225 lightpanda/browser:nightly lightpanda serve --host 0.0.0.0 --port 9225 --advertise-host 127.0.0.1`) | Fastest, lightest. **Single-target CDP** — `webrain_batch` auto-falls back to sequential single-tab reuse (probe detects it). No screenshots/interactive challenges. |
| Pure static HTML, no JS/auth | **no browser** → `webrain_fetch_http` | 10-100× faster than a browser, zero memory. |

> **lightpanda vs obscura batch note:** obscura opens N parallel tabs
> (concurrency = real overlap). lightpanda `serve` holds ONE browser context —
> its 2nd `Target.createTarget` errors `TargetAlreadyLoaded` — so `webrain_batch`
> detects it and runs all URLs sequentially on one reused tab. Same tool call,
> same schema; just no intra-call parallelism. Pick obscura for large parallel
> crawls, lightpanda for footprint/velocity per page (verified: ~2-4s/page on
> mymarket.gr vs ~12s/page on obscura).

> Golden rule: don't guess the browser. `webrain_navigate` returns a
> **`challenge`** field — read it, then pick the browser for the next hop.

---

## 2. Challenge / anti-bot decision tree (do this on EVERY navigate)

After `webrain_navigate(url)`, check `challenge` in the response:

- **`challenge: null`** → page loaded normally → go straight to extraction (§3).
- **`challenge: cloudflare_challenge`** (title "Just a moment…", `_cf_chl*`,
  `cf-turnstile`) or **`blocked`** (403/forbidden) → the page is gated:
  - **obscura CANNOT pass it** (no layout engine → iframe never renders;
    V8 watchdog kills the challenge script — proven: nowsecure, cf-turnstile,
    cf-antibot all fail).
  - **Solution — the "chrome way"** (verified working on cf-antibot):
    1. Run the stealth sidecar:
       ```bash
       python scripts/stealth_solve.py <login_or_challenge_url> --cdp-port 9222 --headed
       ```
       It launches real Chrome + stealth, **waits out the challenge** (do NOT
       reload-spam — that resets the proof), logs in with the page's demo creds
       (or `--creds user:pass`), exports cookies, and keeps Chrome alive.
    2. Re-attach webrain to that Chrome: `CDP_URL=http://127.0.0.1:9222`
       (same browser profile → session/cookies shared).
    3. `webrain_navigate` the protected URL → `challenge: null`,
       authenticated → extract normally.
  - **Non-interactive** Turnstile / basic bot detection → obscura `--stealth`
    may pass (still verify via the `challenge` field).

---

## 3. Extraction tool decision matrix (webrain tools)

| Task says / page is | Tool | Notes |
|---|---|---|
| Structured list (products, results) on current page | `webrain_extract_json(base_selector, fields)` | Schema known → use it directly. |
| **Schema unknown** (from scratch) | `webrain_autoschema` → repeated container, then `webrain_eval` → probe descendant tags/classes, then `webrain_extract_json` | Zero-LLM discovery, no `get_html`. |
| Specific pages / page numbers | `webrain_navigate` per page, or `webrain_batch` for a range | `?page=N` / `/page/N` / `/pagination/N` |
| Many URLs / pages 1..N / catalog | `webrain_batch(op=extract, urls=[...], concurrency=8)` | Fastest for pagination; one call, parallel tabs. |
| **URLs unknown** (discover first) | `webrain_eval` → read pagination `href`s structurally (same-prefix numeric links + next/prev; **no class assumptions**), derive range, then `webrain_batch` | Proven 5-step flow, see §4. |
| Entire site / follow links | `webrain_spider` | BFS/DFS crawl. Args: `allow`/`deny` URL regex (prune /cart, /login, images at crawl time), `retry`, `delay_ms`, `autothrottle` (adaptive per-domain delay, backs off when blocked), `crawldir` (checkpoint/resume for long crawls), `crawl_timeout_secs`. |
| **Site has a sitemap** (discover ALL urls first) | `webrain_sitemap(url)` → `{urls, count}` | Follows robots.txt `Sitemap:` → index → leaf sitemaps → every `<loc>`. Pure HTTP, no browser. Feed the urls into `webrain_batch` for a full crawl. |
| **N interactive sites at once** (load-more / infinite-scroll / form fill) | `webrain_batch(op=interact, urls=[...], interaction=<async JS>, base_selector?, fields?, concurrency=N)` | One call replaces N serial agent loops: the interaction JS runs in PARALLEL tabs, each doing its own waits, then optionally extracts. See `interaction` examples in §4b. |
| **Per-proxy isolation / spread across browsers** | `webrain_batch(op=extract|fetch|interact, urls=[...], cdp_urls=["http://127.0.0.1:9222","http://127.0.0.1:9224",...])` | Round-robins URLs across N CDP backends — each browser = own proxy/cookies/fingerprint = N exit IPs in one call, no subagents. |
| **Persist a batch to disk** | `webrain_batch(..., output="output/my_crawl.json")` | Writes the full payload before returning (`written_to` in response) — survives temp-file GC / dropped responses between turns. |
| Faster/smaller reads on any browser call | `webrain_navigate`/`webrain_batch` args: `network_idle` (wait for no new network), `wait_selector` + `wait_selector_state` (attached/visible/hidden/detached), `disable_resources` (block fonts/images/media), `css_selector` (narrow returned text to one element) | Token + time savers; all default off. |
| Emails, phones, prices, dates, IDs | `webrain_extract_regex` | Built-in patterns. |
| JSON-LD / microdata | `webrain_get_jsonld` | Zero cost. |
| HTML tables | `webrain_table` | |
| Infinite scroll / feed | `webrain_batch(op=interact, urls=[...], interaction=<scroll/click loop>, base_selector, fields)`; or single-page: `webrain_scan` (auto-scroll) then `webrain_extract_json`; or `webrain_click` "Load More" + re-extract | Dynamic pages: wait for JS to hydrate before extracting. `op=interact` is the parallel multi-page path. |
| Search engines | `webrain_search` | No browser needed — duckduckgo (default) / google / bing via plain HTTP. `brave` is JS-rendered (SPA shell) — use `webrain_navigate("https://search.brave.com/search?q=...")` for real Brave results. |
| Static page, no browser | `webrain_fetch_http(url)` | |
| Relevance filter on extracted items | `webrain_bm25(query, items)` | Keep top-k. |

**Extraction cascade (cost-ascending, like crawl4ai):** CSS schema → XPath →
Regex → JSON-LD → (LLM last). If `webrain_extract_json` returns 0 items:
1. Check the page really loaded (`challenge`/title not a block page).
2. Re-run `webrain_autoschema` + field probe (selectors may not match DOM).
3. If page is a JS SPA that renders late → scroll/wait, then retry.

## Checkpoints on `webrain_spider` — when and how

`webrain_spider(crawldir="<dir>", checkpoint_every=N)` persists `{queue, seen}`
to `<dir>/checkpoint.json` every N pages, and a later call with the SAME
`crawldir` resumes from where it stopped. The checkpoint is deleted only on a
clean (queue-drained) finish; a capped/timeout/error crawl KEEPS it.

**Use a checkpoint when:**
- The crawl is big enough that it might not finish in one call — roughly
  `max_pages × per-page-cost > your timeout budget`. On obscura a page is
  ~8-13s, so `max_pages=30`+ is already a multi-minute crawl: set a
  `crawldir`.
- You're being polite/throttled (`autothrottle` + `delay_ms` slow it down) and
  a hard `crawl_timeout_secs` will cut it off mid-way.
- You want to iterate: run once to seed the queue, then re-run with a bigger
  `max_pages` to continue — the second run resumes, it doesn't restart.

**How (typical flow for a long crawl):**
1. First call: `webrain_spider(seed_url, crawldir="output/site_crawl", max_pages=50, checkpoint_every=10, crawl_timeout_secs=<your call budget>, ...)`. Run it; if it returns all 50, done. If it stops early (timeout/cap), the checkpoint is left on disk.
2. Continue: call `webrain_spider(seed_url, crawldir="output/site_crawl", max_pages=200, ...)` — same `crawldir` → resumes the queue, skips already-visited URLs. Repeat raising `max_pages` until the crawl reports a clean finish (checkpoint auto-deleted).
3. Each response returns `stats: {elapsed_ms, pages_ok, pages_err, page_ms_total}` — use it to judge whether to continue (pages_ok growing, queue not drained) or stop.

**Gotchas:**
- Use the SAME `crawldir` string to resume; a different dir = a fresh crawl.
- `max_pages` still caps each *call*, not the whole resume chain — raise it per call to go further.
- The checkpoint does NOT store extracted items, only crawl state (queue + seen) — persist products separately via `output="..."` on a batch, or read each page as you go.
- Autothrottle's learned delays are per-crawl (not checkpointed) — a resumed crawl starts throttling fresh.

**HTML is the LAST resort — never default to it.** `webrain_snapshot`
(text+elements), `webrain_clean` (stripped text), `webrain_eval` (sync JS),
and `webrain_extract_json`/`table`/`regex` (structure) all return page content
far cheaper than `webrain_get_html` (raw markup, token-heavy, unreadable to an
LLM). Call `webrain_get_html` ONLY when the task explicitly asks for HTML
markup (a scraper spec, a tag/attribute audit) — and if you do, tell the user
you're pulling HTML and why. Otherwise: snapshot first, always.

---

## 4. From-scratch discovery workflow (schema + URLs unknown)

Proven on scrapingcourse pagination (147 items, identical to manual runs):

```
STEP 1: webrain_navigate(seed_url)
STEP 2: webrain_eval → collect pagination hrefs:
        links whose path = current_path + '/' + <numeric>, plus next/prev labels
        (NO hardcoded .px-2/.next-page — derive structurally)
        → max page → build URLs [seed, .../2, .../N]
STEP 3: webrain_autoschema → repeated container selector (e.g. div.product-item)
STEP 4: webrain_eval → probe first container: descendant tag/class + sample text
        → build fields [{name, selector, type: text|attr}]
STEP 5: webrain_batch(op=extract, urls=<discovered>, base_selector, fields,
        concurrency=8) → aggregate
STEP 6: done(summary="Extracted N items across M pages")
```

**Never** hardcode site-specific selectors from memory — always discover via
autoschema/eval unless the schema is already known/verified.

---

## 4b. `op=interact` — parallel interaction for N interactive sites

Run one async JS interaction in PARALLEL tabs (one tab per URL, semaphore-bounded),
then optionally extract a schema. Replaces N serial agent loops for N
load-more / infinite-scroll / form-fill pages.

```
webrain_batch(
  op="interact",
  urls=[siteA, siteB, ...],
  interaction="(async () => { const btn = document.querySelector('#load-more-btn');
                 for (let i=0; i<10 && btn && btn.offsetParent!==null; i++) {
                   btn.click(); await new Promise(r=>setTimeout(r,400)); }
                 window.scrollTo(0, document.body.scrollHeight);
                 await new Promise(r=>setTimeout(r,500)); return 'done'; })()",
  base_selector="div.product-item.flex",        // optional: extract after
  fields=[{name, selector, type}],              // optional
  concurrency=8,
  output="output/crawl.json")                   // optional: persist to disk
```

- `interaction` is **async JS, side-effects only** (clicks/scrolls/fills) — it does
  its own waits. Return value ignored.
- `base_selector` + `fields` optional: if given, a CSS-schema extract runs after
  the interaction on each tab.
- `cdp_urls` (list) → round-robin across N CDP backends (per-proxy isolation:
  each browser = own proxy/cookies/fingerprint). Use when a site rate-limits by
  IP or you need separate cookie jars — one call, N exit IPs, no subagents.
  When you also need full isolation of *independent interactive sites*, combine
  with `webrain_open_session` per target + `Mcp-Session-Id` header routing.
- Verified live on scrapingcourse.com: button-click → 132 products,
  infinite-scrolling → products, in a single parallel call.

---

## 5. Login / gated flows

```
STEP 1: webrain_navigate(login_url)
STEP 2: if challenge field set → chrome way (§2), else continue
STEP 3: webrain_type(email_index, email); webrain_type(password_index, pwd)
        (find field indices from navigate/snapshot elements, or webrain_a11y)
STEP 4: webrain_click(submit_index)
STEP 5: webrain_navigate(protected_url) → verify challenge:null + auth marker
        (e.g. "Logout", "Welcome, <user>")
STEP 6: extract (§3) → done
```

---

## 6. Anti-bot reality check (what each backend can/cannot do — verified)

| Backend | Managed JS challenge (Just a moment) | Interactive Turnstile | Non-interactive Turnstile | Screenshot / render |
|---|---|---|---|---|
| obscura `--stealth` (lean build) | ❌ V8 crashes (`YS is not a function`) | ❌ no iframe, no token | ~ (best effort) | ❌ no paint engine |
| real Chrome + `stealth_solve.py` (headed) | ✅ | ⚠️ needs solve service/click | ✅ | ✅ |
| lightpanda | ❌ | ❌ | ~ | ❌ |

When in doubt: navigate, read the `challenge` field, choose accordingly.

---

## 7. Task-derived lessons (verified on a full scrapingcourse.com crawl)

These come from running a real 6-source crawl (653 products) and shaving the
latency that cost an agent the most.

**Internal-link discovery is now free.** `webrain_navigate`/`snapshot` return a
`links` field — deduped same-origin hrefs (≤200). **Scrapling LinkExtractor-quality
cleaning:** URLs are canonicalized (trailing-slash normalized), fragments
stripped, non-http(s) schemes dropped (mailto:, javascript:, tel:), 50+
content-obvious extensions filtered (images, fonts, pdf, archives, css/js, ico,
video/audio), and `area[href]` is supported. Verified: scrapingcourse /ecommerce →
28 clean links (product + page), down from ~50 raw containing product images,
add-to-cart fragments, and ico/css/js noise.
For "crawl site + internal links", navigate the seed and read `links` directly;
no separate `webrain_eval` for hrefs. Expand only the links you need (pagination
patterns etc.), capped at your URL budget.

**Read `data`, not `text`, from batch.** `webrain_batch(op=extract|interact)`
now returns each result's products as a parsed `data` array (same shape as
single-page `webrain_extract_json`). Parse nothing; `text` is kept only for
backward compat. Previously the products were a JSON *string* inside `text`,
which cost repeated `data`/`text` confusion.

**Load-more / infinite-scroll shortcut (fastest path).** These pages back the
button/observer with a plain endpoint (scrapingcourse: `/ajax/products?offset=N`).
Find it by grepping the page's own scripts via `webrain_eval` for `/ajax/`.
Then `webrain_batch(op=extract, urls=[...offset=0,10,20...], base_selector,
fields)` directly — no click/scroll loop, one call. The endpoint often returns
a sliding window (offset=0→1-12, offset=10→11-22…): dedupe by url/name. This
beat the interaction loop by ~3× and avoided observer/scroll races entirely.

**`webrain_eval` + async on obscura.** Browser-level `Runtime.evaluate` does not
reliably await async JS on obscura — an async IIFE returns `null`. For async
work (fetch loops, waits), use `webrain_batch(op=interact, ...)`: its
interaction runs in a session where `await` resolves. Sync JS on `webrain_eval`
is fine.

**`webrain_type` index = the SAME list as `navigate`/`snapshot` elements.**
`type_text` now uses the identical selector as `ELEMENTS_JS`/`click`
(`a, button, input, select, textarea, [role="button"]`), so the index you read
from a snapshot maps 1:1 to `webrain_type`. Before the fix it enumerated only
`input/textarea/select` — on a page whose first interactive element is a link
(like scrapingcourse's CSRF login: `#logo-link` first), the index pointed at
the wrong field. If a type lands in the wrong box, re-check the page's FIRST
interactive element — it may be a logo link or button that shifts indices.
Hidden inputs (e.g. CSRF tokens) also occupy an index.

**Chrome sidecar solves managed-CF + login, but NOT embedded Turnstile
widgets.** `scripts/stealth_solve.py` waits out title-based interstitials
("Just a moment…"), then fills the login form. It works for `/login/cf-antibot`
(full solve → dashboard) and plain/CSRF login. But a page that embeds a
Turnstile *checkbox widget* (scrapingcourse `/login/cf-turnstile`) has a normal
title from load, so the sidecar skips the wait and submits without a token →
`Forbidden`. Solving that needs a real click on the widget's iframe checkbox
(manual or a click-capable automation loop). `cf_clearance` from one challenge
does not transfer to a differently-solved page.

**`obscura::console` ERROR lines are page noise, not failures.** obscura logs
every page `console.error(...)` at ERROR level under the `obscura::console`
target. Its Web Worker shim (`globalThis.Worker` in bootstrap.js) runs page
worker code via `new Function(...)` and logs `Worker error: <msg>` when that
worker script throws a ReferenceError (e.g. `i is not defined`) — obscura has
no real Worker global, so page code that spawns `new Worker()` typically
fails here. This is benign: it doesn't affect scraping. Judge success by the
`challenge` field and the extracted `data`, not by scanning docker console
ERRORs.

**`obscura::console` also logs third-party widget failures (benign).** Pages
with accessibility/livechat/tracker widgets (e.g. `cdn.equalweb.com
accessibility.js`, seen on any domain) log `Dynamic script error: Cannot read
properties of undefined` — those widgets need a full DOM obscura doesn't
provide. Same category as Worker errors: noise, not a scrape failure. If a page
extracts cleanly (`data` non-empty, `challenge` null), ignore them.

**obscura in Docker must bind `0.0.0.0`.** Default `obscura serve` binds
`127.0.0.1` *inside* the container — the published `-p HOST:9222` port accepts
TCP but answers nothing (docker-proxy → loopback mismatch). Start with
`serve --port 9222 --host 0.0.0.0 --stealth` and point webrain at
`ws://<host>:<mapped>/devtools/browser`. A curl `http://host:port/json/version`
probe confirms reachability before any crawl.

