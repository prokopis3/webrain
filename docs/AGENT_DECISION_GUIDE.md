# Webrain — Agent Decision Guide (LLM reads this before using MCP tools)

How to choose the optimal path for every scraping task: **which browser**, **how
to handle a challenge**, and **which extraction tool**. Built from the
browsemind extraction guides + verified live on scrapingcourse.com.

> **Tool naming.** This guide predates the 17-tool consolidation and mostly
> uses the legacy one-action names (`webrain_fetch_http`,
> `webrain_extract_json`, `webrain_spider`, `webrain_click`, …). Those still
> dispatch, but the consolidated surface is the primary dialect:
> `webrain_scrape`, `webrain_extract(mode=schema)`, `webrain_crawl(mode=spider)`,
> `webrain_interact(action=click)`, `webrain_observe(what=…)`, … The runtime
> `webrain_guide` tool and the [Tools Reference](/reference/tools) carry the
> exact legacy → consolidated map.

---

## 1. Browser selection (set `CDP_URL` / pick the backend)

| Situation | Browser | Why |
|---|---|---|
| **Interactive Material / heavy SPA** (Google Flights, calendars, Material dropdowns/comboboxes, segmented controls) | **real Chrome**, routed per-tab via `cdp_urls:["http://127.0.0.1:9222"]` | Material/Google widgets open on `mousedown`/pointer hit-testing and read computed styles — only a real layout+paint engine works. **Never obscura or lightpanda** for interactive Material: obscura v0.2.0 rendering is not full Chromium parity, and lightpanda has no layout engine. |
| **Cloudflare / Turnstile / CAPTCHA / any interactive challenge**, or need **real** screenshots / pixel rendering | **real Chrome + persistent profile + session** (`webrain launch` / `webrain_session(op=login)`) | Only a browser with a human-solvable session clears these. Obscura's render build **does** paint (PNG/PDF verified on v0.2.5), but its stealth cannot complete an interactive challenge — never read "it renders" as "it solves". |
| JS-rendered pages **without** a challenge, high-concurrency batch scraping | **obscura** (docker, `--stealth`) | Fast, light, no Chrome overhead. Multi-tab: `webrain_batch` runs parallel tabs. v0.2.0+ render builds screenshot + PDF; no interactive Material (not full layout parity). |
| Lightweight / minimal footprint, want a **real AX tree + semantic tree**, no rendering needed | **lightpanda** (`docker run -d --rm --name lightpanda -p 9225:9225 lightpanda/browser:nightly lightpanda serve --host 0.0.0.0 --port 9225 --advertise-host 127.0.0.1`) | Fastest, lightest, **real a11y** (`Accessibility.getFullAXTree` + `LP.getSemanticTree`). **Single-target per CONNECTION** — a batch on one endpoint serializes (probe detects it, `single_target: true`), but the process serves many connections: repeat the endpoint in `cdp_urls` for parallel pages, up to `--cdp-max-connections` (16). ⚠️ No layout engine: `captureScreenshot` returns a **fake placeholder PNG** (silently wrong, not an error), and interactive Material won't work. |
| Pure static HTML, no JS/auth | **no browser** → `webrain_scrape` | 10-100× faster than a browser, zero memory. |

> **Batch parallelism is per CONNECTION, not per process — this is the rule.**
> Real Chrome and obscura open N parallel tabs on one connection, so
> `concurrency` is the lever there. **lightpanda is single-target per CDP
> connection**: its `Target.createTarget` returns `TargetAlreadyLoaded` once the
> context holds a target, and a second `Target.createBrowserContext` is refused
> with *"Cannot have more than one browser context at a time"*
> (`src/server/cdp/domains/target.zig`; `CDP.zig` keeps one `target_id` —
> *"we only allow 1 open page at a time"*). So a single endpoint runs URLs
> sequentially and the reply says so: `single_target: true,
> effective_concurrency: 1`. **No `concurrency` value changes that.**
>
> **But one lightpanda process serves MANY connections.** `serve` builds
> `Worker.Pool.init(allocator, config.maxConnections())` and spawns a worker
> thread — its own browser context and page — per CDP WebSocket, capped by
> `--cdp-max-connections` (**default 16**). Verified on `src/server/Server.zig`
> (`spawnWorker` → `Worker.Pool.acquire`) and measured against ONE container:
>
> | connections to the same endpoint | URLs | wall | `effective_concurrency` |
> |---|---|---|---|
> | 1 | 8 | 6.15 s | 1 (wall ≈ Σ page times) |
> | 4 | 8 | **1.17 s** | 4 |
> | 8 | 16 | **4.19 s** (Σ 20.1 s) | 8 |
>
> So the cheap lever is **repeat the same endpoint in `cdp_urls`**, N times, up
> to `--cdp-max-connections`.
>
> **Connections are not the whole story: workers in one process SHARE the HTTP
> pool.** `Network` (on `App`) owns `--http-max-concurrent` (default 40) and
> `--http-max-host-open` (**default 6 per host**), so 8 workers all pulling
> Wikipedia queue behind 6 per-host slots and every page slows down. Measured on
> 25 URLs: 8 connections to one container = **9.6 s** (Σ page times 60.9 s) vs
> **3.5 s** across 10 separate containers (Σ 25.0 s). Separate processes each
> get their own pool, plus their own profile, cookies and exit IP.
>
> **So pick by what you need:** a few extra connections (≈4 is the sweet spot on
> this box) when the URLs are spread across many hosts; separate processes when
> many URLs share a host, or you need per-IP isolation; and either way the real
> ceiling is CPU — Docker here has `NCPU=4`, so ~4 productive workers per 4
> cores. Connections cost nothing in RAM; a container costs ~85 MB and boot
> time.
>
> **Lightpanda Cloud works with no container at all.** Its endpoint is a raw
> `wss://` URL (`wss://euwest.cloud.lightpanda.io/ws?token=<token>&browser=lightpanda`)
> and `CDP_URL` / `cdp_urls` accept `ws://` and `wss://` directly — passed
> through, never probed for `/json/version`. Same rules: each connection is one
> page, so list the cloud endpoint N times for N concurrent pages, up to your
> plan's connection cap. Measured: 1 connection = 25 URLs in 16.5 s (Σ page
> times 14.7 s, i.e. fully serial); per-page cost is close to local, the
> handshake is what costs ~1.8 s. Treat the token as a credential: it is
> billable, never commit it or paste it into a report.
>
> **Do not mix a slow remote backend into a fast local pool.** Round-robin
> splits URLs evenly regardless of per-backend speed, so the slowest member sets
> the wall clock. Measured on 25 URLs: 10 local containers = 3.5 s, cloud alone
> = 16.5 s, **cloud + those same 10 local containers = 8.3 s** — worse than the
> local pool alone, because the cloud connection's 3 URLs (one took 3.5 s) cost
> more than the entire local run. Give a remote endpoint its own call, or rank
> backends by measured per-page time before assigning shares.
>
> Pick obscura for large parallel crawls, lightpanda for footprint and
> per-page velocity (verified: ~2-4s/page on mymarket.gr vs ~12s/page on
> obscura), N lightpanda containers when you want that velocity at scale.

### a11y (observe what=a11y)
Google/Material widgets are often NOT `button`: dropdowns are `combobox`, menu
items `option`, segmented controls `radio`/`tab`. `filter` matches name OR value
OR css_path (case-insensitive); `role` is a substring match, so `role=button`
also finds `pushbutton`/`radiobutton`. If `role=<x>` returns [], drop the role
filter and `filter` on the visible label text instead. If the whole tree is
empty, the page never rendered (check `webrain_navigate`'s challenge/consent
field — e.g. obscura on Google Flights sat on the consent page, so a11y found
nothing). Known upstream gap: obscura's AX role map has no `option` arm
(`role="option"` → `generic`); don't rely on obscura for option roles.

> Golden rule: don't guess the browser. `webrain_navigate` returns a
> **`challenge`** field — read it, then pick the browser for the next hop.

---

## 2. Challenge / anti-bot decision tree (do this on EVERY navigate)

After `webrain_navigate(url)`, check `challenge` in the response:

- **`challenge: null`** → page loaded normally → go straight to extraction (§3).
- **`crippled: true`** (title "Attention Required!" / "you have been blocked") →
  a HARD block, NOT a challenge — a WAF rule (usually IP/ASN) refused outright.
  Headless Chrome is detectable and triggers it. Fix: real HEADED Chrome +
  persistent profile + `--disable-blink-features=AutomationControlled`; if the
  same IP still blocks, route a clean-IP proxy (`proxy` param) or report — do
  NOT loop. Scope matters: a `/rss/` endpoint can be blocked while the homepage
  loads; the real feed may be off-domain (FeedBurner) — `eval` the
  `link[rel="alternate"]` tags or spider `(?i)rss|feed|atom` to find it, then
  fetch it plain-HTTP (not blocked).
- **`challenge: cloudflare_challenge`** (title "Just a moment…", `_cf_chl*`,
  `cf-turnstile`) or **`blocked`** (403/forbidden) → the page is gated:
  - **obscura CANNOT pass it** (no layout engine → iframe never renders;
    V8 watchdog kills the challenge script — proven: nowsecure, cf-turnstile,
    cf-antibot all fail).
  - **Solution — persistent profile + real Chrome + session** (native):
    1. Launch real Chrome on a persistent profile: `webrain launch <service>
       <profile> <login_url>` (or `webrain_session(op=login, ...)` for native
       vault + TOTP — it auto-waits challenges and claims
       Turnstile/reCAPTCHA/hCaptcha widgets; do NOT reload-spam — that resets
       the proof).
    2. On a 2FA/approval gate, login returns `waiting_for_human:true` — the
       human acts in the headed browser, then call login again.
    3. Re-attach webrain to that Chrome: `CDP_URL=http://127.0.0.1:9222` (same
       browser profile → session/cookies shared), or
       `webrain_session(op=open, cdp_url=...)`.
    4. `webrain_navigate` the protected URL → `challenge: null`, authenticated →
       extract normally.
    Interactive Turnstile/hCaptcha the native path can't claim need a human in
    the headed browser (`waiting_for_human:true`).
  - **Non-interactive** Turnstile / basic bot detection → obscura `--stealth`
    may pass (still verify via the `challenge` field).

### 2c. Interactive challenge → which input primitive

Every widget below needs **trusted CDP input**; a JS `.click()` or a synthetic
`MouseEvent` has `isTrusted=false` and is discarded. Coordinates come from
`webrain_observe what=screenshot` (pixels == viewport CSS px at DPR 1).

| Widget | Primitive |
|---|---|
| reCAPTCHA v2 / hCaptcha / Turnstile checkbox | `interact click_coords` at the checkbox (~top-left of the widget, **not** its centre) |
| Image grid ("select all bicycles") | repeated `click_coords` on tile centres, then the Verify button |
| **"Press & Hold to confirm you are a human"** (PerimeterX/HUMAN) | `interact hold x,y,ms` — a click or a short drag never fires; hold **≥ 6 s** |
| Slider / puzzle drag (GeeTest, DataDome, AWS WAF) | `interact drag x1,y1,x2,y2` — eased path; `ms` = dwell on the handle before moving |
| Text or audio challenge | `interact type` with `ms` (real per-key), then `press Enter` |
| Prefilled field blocking submit | `press ctrl+a`, then `press Backspace` |
| Challenge inside a scrolling modal | `scroll x,y,delta` — wheels the element **under** that point |

Two traps that look like a broken widget:

- **Input is dropped on a background/occluded tab.** Measured on a PerimeterX
  page: 12 `mouseMoved` landed, **0** `mousePressed`, with
  `document.visibilityState === "hidden"`. webrain now sends
  `Page.bringToFront` + `Emulation.setFocusEmulationEnabled` at attach; if you
  drive CDP yourself, do the same.
- **A challenge is not a page.** A solved widget issues a token
  (`#g-recaptcha-response`, `_px3`/`_px2` cookie, `cf_clearance`) — read that,
  not the green checkmark.

No interaction exists for **Akamai, Cloudflare managed, DataDome passive**: they
grade the whole session. The fix is profile + session (§2b), never more clicks.

---

## 2b. "It renders but never loads" — hydration check

Do this before blaming the site. `webrain_navigate` returns when `readyState`
is `complete`, which is BEFORE the SPA mounts, so an immediate probe can read a
dead shell on a perfectly healthy page. Re-probe after ~1s, then:

```js
const el = document.querySelector('input');                    // a real control
Object.keys(el).filter(k => k.startsWith('__react')).length    // 0 = not hydrated
Object.keys(el).some(k => k.startsWith('__react'))             // true = React attrs
```

Gate on the **interaction**, not the markup: a page whose submit button is still
`disabled` and whose list is empty has not mounted, even with the right title and
`challenge: null`.

Measured cause on `https://www.ab.gr/storelocator` (2026-10-24, real Chrome): the
injected stealth shim, not the site.

- A fake `navigator.plugins` (built with `Object.create(PluginArray.prototype)`)
  made iteration throw `TypeError: Illegal invocation` in page code. Fixed:
  spoof only when the browser reports none, and set `length` via
  `Object.defineProperty`.
- The NOISE block (default ON) replaces `Object.defineProperty` globally and hooks
  `CanvasRenderingContext2D.getImageData`, `AudioBuffer.getChannelData`,
  `HTMLMediaElement.canPlayType`. With NOISE on, Next.js aborted hydration
  (`REJ Cannot read properties of undefined (reading 'getInitialProps')`, zero
  react keys, 558-char body); with `WEBRAIN_STEALTH_NOISE=0` the same browser
  hydrated (3 react keys, button enabled, 4919-char body).

Procedure when you hit this:

1. `WEBRAIN_STEALTH_NOISE=0` in the process env that owns the CDP connection
   (read at attach — restart an MCP server to apply it).
2. Open a FRESH tab. `Page.addScriptToEvaluateOnNewDocument` registrations last
   for the target's lifetime; re-navigating the same tab silently keeps the old
   shim, which is why an env-var change can appear to do nothing.
3. Compare against a hand-opened tab of the same URL. If the human tab works and
   ours does not, it is our injection — never report the site as frozen.

Prefer real Chrome + a real fingerprint on real sites: the fingerprint noise
buying Cloudflare coverage is exactly what costs SPA compatibility.

---

## 3. Extraction tool decision matrix (webrain tools)

| Task says / page is | Tool | Notes |
|---|---|---|
| Structured list (products, results) on current page | `webrain_extract(mode=schema, base_selector, fields)` | Schema known → use it directly. |
| **Schema unknown** (from scratch) | `webrain_extract(mode=autoschema)` → repeated container, then `webrain_eval` → probe descendant tags/classes, then `webrain_extract(mode=schema)` | Zero-LLM discovery, no `observe(what=html)`. |
| Specific pages / page numbers | `webrain_navigate` per page, or `webrain_batch` for a range | `?page=N` / `/page/N` / `/pagination/N` |
| Many URLs / pages 1..N / catalog | `webrain_batch(op=extract, urls=[...], concurrency=8)` | Fastest for pagination; one call, parallel tabs. |
| **URLs unknown** (discover first) | `webrain_eval` → read pagination `href`s structurally (same-prefix numeric links + next/prev; **no class assumptions**), derive range, then `webrain_batch` | Proven 5-step flow, see §4. |
| Entire site / follow links | `webrain_crawl(mode=spider)` | BFS/DFS crawl. Args: `allow`/`deny` URL regex (prune /cart, /login, images at crawl time), `retry`, `delay_ms`, `autothrottle` (adaptive per-domain delay, backs off when blocked), `crawldir` (checkpoint/resume for long crawls), `crawl_timeout_secs`. |
| **Site has a sitemap** (discover ALL urls first) | `webrain_crawl(mode=sitemap, url)` → `{urls, count}` | Follows robots.txt `Sitemap:` → index → leaf sitemaps → every `<loc>`. Pure HTTP, no browser. Feed the urls into `webrain_batch` for a full crawl. |
| **N interactive sites at once** (load-more / infinite-scroll / form fill) | `webrain_batch(op=interact, urls=[...], interaction=<async JS>, base_selector?, fields?, concurrency=N)` | One call replaces N serial agent loops: the interaction JS runs in PARALLEL tabs, each doing its own waits, then optionally extracts. See `interaction` examples in §4b. |
| **Per-proxy isolation / spread across browsers** | `webrain_batch(op=extract|fetch|interact, urls=[...], cdp_urls=["http://127.0.0.1:9222","http://127.0.0.1:9224",...])` | Round-robins URLs across N CDP backends — each browser = own proxy/cookies/fingerprint = N exit IPs in one call, no subagents. |
| **N independent sites / more throughput / different engines** | **delegate** — spawn one subagent per browser/site-group, each with its own `CDP_URL` (see §4c) | One browser already parallelizes inside via `webrain_batch concurrency=N`; delegate *across* browsers for scale. |
| **Huge site-wide crawl** | **delegate** — shard by subdomain/section, one `webrain_crawl(mode=spider)` per subagent (each its own `crawldir` for checkpoint/resume) | See §4c. |
| **Persist a batch to disk** | `webrain_batch(..., output="output/my_crawl.json")` | Writes the full payload before returning (`written_to` in response) — survives temp-file GC / dropped responses between turns. |
| Faster/smaller reads on any browser call | `webrain_navigate`/`webrain_batch` args: `network_idle` (wait for no new network), `wait_selector` + `wait_selector_state` (attached/visible/hidden/detached), `disable_resources` (block fonts/images/media), `css_selector` (narrow returned text to one element) | Token + time savers; all default off. |
| Emails, phones, prices, dates, IDs | `webrain_extract(mode=regex)` | Built-in patterns. |
| JSON-LD / microdata | `webrain_extract(mode=jsonld)` | Zero cost. |
| HTML tables | `webrain_extract(mode=table)` | |
| Infinite scroll / feed | `webrain_batch(op=interact, urls=[...], interaction=<scroll/click loop>, base_selector, fields)`; or single-page: `webrain_crawl(mode=scan)` (auto-scroll) then `webrain_extract(mode=schema)`; or `webrain_interact(action=click)` "Load More" + re-extract | Dynamic pages: wait for JS to hydrate before extracting. `op=interact` is the parallel multi-page path. |
| Search engines | `webrain_serp` (structured JSON: position/title/url/snippet — duckduckgo/bing/yahoo/baidu/yandex need no browser; google and brave use the connected CDP engine) · `webrain_search` (navigate to the results page) | `webrain_serp` dedupes, paginates (`page`), pins an **en-US market when no `region`** is set (a GeoIP'd IP otherwise serves localized garbage), and falls back across providers; `engine=auto` fetches **duckduckgo + bing concurrently over plain HTTP** (no browser), then **tops up from baidu, yandex and yahoo** while the reply is short — yahoo last, because it is the only one that pages to a deep limit (measured `--limit 50 --http`: 25 rows from ddg+bing alone, 50 with the top-up) — and — when a CDP engine is attached — also renders google + brave and merges everything; results are **relevance-filtered** (unrelated junk pages from flagged/GeoIP'd IPs are dropped) and deduped, with a `per_engine` breakdown in the reply. Over plain HTTP only duckduckgo/bing/yahoo answer reliably: **baidu** serves ~7 organic rows and then 302s to `wappass.baidu.com` (百度安全验证 image captcha), and **yandex** serves a SmartCaptcha page from a datacenter IP — both come back as `skipped` with the reason, never as an empty result set. A real Chrome with a warm profile passes the baidu check; a fresh or headless profile does not (measured 2026-10-26). google is JS-gated and brave is an SPA shell, so ask for `engine=google` / `engine=brave` explicitly when you need them — those render in the connected CDP engine (auto-launched persistent-profile Chrome for google, hard-refresh + humanized flow, optional 2captcha solve via `WEBRAIN_2CAPTCHA_KEY`, optional **serpapi.com fallback via `SERPAPI_API_KEY`**). Pass `proxy` to route the HTTP engines and the google launch through an HTTP(S)/SOCKS proxy — a clean-IP proxy is the reliable way to beat GeoIP-locked bing/google (they ignore market params on flagged/rotating IPs). |
| Static page, no browser | `webrain_scrape(url)` | |
| Relevance filter on extracted items | `webrain_extract(mode=bm25, query, items)` | Keep top-k. |

**Extraction cascade (cost-ascending, like crawl4ai):** CSS schema → XPath →
Regex → JSON-LD → (LLM last). If `webrain_extract(mode=schema)` returns 0 items:
1. Check the page really loaded (`challenge`/title not a block page).
2. Re-run `webrain_extract(mode=autoschema)` + field probe (selectors may not match DOM).
3. If page is a JS SPA that renders late → scroll/wait, then retry.

## Checkpoints on `webrain_crawl` (mode=spider) — when and how

`webrain_crawl(mode=spider, crawldir="<dir>", checkpoint_every=N)` persists
`{queue, seen, results}` atomically (tmp+rename) to `<dir>/checkpoint.json`
every N NEW pages. A later call with the SAME `crawldir` resumes from where it
stopped — it returns prior + new results and never re-fetches a seen URL. The
checkpoint is deleted only on a clean (queue-drained) finish; a
capped/timeout/error crawl KEEPS it (plus a final snapshot). `max_pages` counts
the WHOLE crawldir across resumes (prior pages included) — raise it to fetch
more. On multi-target backends (real Chrome/obscura) the spider loads pages in
`concurrency` (default 4) parallel tabs; lightpanda / `concurrency: 1` = serial.

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
1. First call: `webrain_crawl(mode=spider, seed_url, crawldir="output/site_crawl", max_pages=50, checkpoint_every=10, crawl_timeout_secs=<your call budget>, ...)`. Run it; if it returns all 50, done. If it stops early (timeout/cap), the checkpoint is left on disk.
2. Continue: call `webrain_crawl(mode=spider, seed_url, crawldir="output/site_crawl", max_pages=200, ...)` — same `crawldir` → resumes the queue, skips already-visited URLs. Repeat raising `max_pages` until the crawl reports a clean finish (checkpoint auto-deleted).
3. Each response returns `stats: {elapsed_ms, pages_ok, pages_err, page_ms_total}` — use it to judge whether to continue (pages_ok growing, queue not drained) or stop.

**Gotchas:**
- Use the SAME `crawldir` string to resume; a different dir = a fresh crawl.
- `max_pages` caps the whole crawldir (prior resumed pages count) — a resume
  with the same `max_pages` fetches nothing new; raise it to go further.
- A resume replays the already-collected `results` in its response (crash
  safety costs a little re-sending) — dedupe by URL if you only want the new
  pages.
- The checkpoint does NOT store extracted items, only crawl state (queue + seen) — persist products separately via `output="..."` on a batch, or read each page as you go.
- Autothrottle's learned delays are per-crawl (not checkpointed) — a resumed crawl starts throttling fresh.

**HTML is the LAST resort — never default to it.** `webrain_observe(what=state)`
(text+elements), `webrain_observe(what=clean)` (stripped text), `webrain_eval`
(sync JS), and `webrain_extract(mode=schema|table|regex)` (structure) all return
page content far cheaper than `webrain_observe(what=html)` (raw markup,
token-heavy, unreadable to an LLM). Call `webrain_observe(what=html)` ONLY when
the task explicitly asks for HTML markup (a scraper spec, a tag/attribute
audit) — and if you do, tell the user you're pulling HTML and why. Otherwise:
observe state first, always.

---

## 4. From-scratch discovery workflow (schema + URLs unknown)

Proven on scrapingcourse pagination (147 items, identical to manual runs):

```
STEP 1: webrain_navigate(seed_url)
STEP 2: webrain_eval → collect pagination hrefs:
        links whose path = current_path + '/' + <numeric>, plus next/prev labels
        (NO hardcoded .px-2/.next-page — derive structurally)
        → max page → build URLs [seed, .../2, .../N]
STEP 3: webrain_extract(mode=autoschema) → repeated container selector (e.g. div.product-item)
STEP 4: webrain_eval → probe first container: descendant tag/class + sample text
        → build fields [{name, selector, type: text|attr}]
STEP 5: webrain_batch(op=extract, urls=<discovered>, base_selector, fields,
        concurrency=8) → aggregate
STEP 6: done(summary="Extracted N items across M pages")
```

**Never** hardcode site-specific selectors from memory — always discover via
autoschema/eval unless the schema is already known/verified.

### Pagination type decision tree (identify BEFORE picking a pipeline)

```
Page loaded → observe the pagination area:
  A) Numbered links (1,2,3…)?       → discover links → webrain_crawl(mode=validate)
                                    → webrain_batch (State I/O pattern)
  B) "Next" button only, no numbers?→ session click loop: click(Next) → extract → repeat
  C) "Load More" / "Show More"?     → click loop (webrain_batch op=interact for many)
  D) Infinite scroll / feed?        → webrain_crawl(mode=scan) (or op=interact scroll loop)
  E) Direct URL pattern (/page/N)?  → construct URLs → webrain_batch
  F) Unknown / complex?             → webrain_crawl(mode=spider) (auto-discovers)
```

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
  with `webrain_session(op=open)` per target + `Mcp-Session-Id` header routing.
- Verified live on scrapingcourse.com: button-click → 132 products,
  infinite-scrolling → products, in a single parallel call.

---

## 4c. Multi-agent delegation (orchestrator pattern)

webrain's MCP server creates a **per-connection session** per client — so you
(the orchestrator) can spawn subagents, give each its own browser via
`CDP_URL`, and they run fully in parallel. This is the scaling lever beyond a
single browser's `webrain_batch concurrency=N`.

**Delegate when you hit ANY of:**
1. **Many independent URLs/pages/sites** — shard the URL list across subagents
   (each runs `webrain_batch(op=extract, urls=<its shard>)`). One browser
   already parallelizes in-process; delegate *across browsers* when you need
   more throughput or different engines.
2. **Different engines/roles** — challenge-heavy pages → Chrome subagent
   (9222), bulk pages → obscura subagent (9224), static → `webrain_scrape`
   subagent (no CDP). Matches the browser matrix (§1).
3. **Per-proxy / per-IP isolation** — one `CDP_URL` per subagent = each
   browser carries its own proxy/cookies/fingerprint = N exit IPs at once.
4. **Huge site-wide crawl** — shard by subdomain/section; one `webrain_crawl(mode=spider)`
   per subagent (each with its own `crawldir` for checkpoint/resume).
5. **Discovery overlapping extraction** — one subagent discovers schema/URLs on
   a new site while others extract from already-known sites.

**Delegation is the LAST parallel lever.** A single browser ALREADY
parallelizes in-process (`webrain_batch concurrency=N`, `cdp_urls` round-robin
across N backends in one call). Spawn subagents only when in-browser
parallelism is exhausted (e.g. >~100 URLs, each already batched) or you need
DIFFERENT browsers/proxies. Delegating what one `webrain_batch` handles is
wasted orchestration (browsemind Rule B: parallel over sequential; Rule E:
reduce browser launches).

**Delegate by PATTERN (shard the task, not the steps):**

| Task pattern | Default (one agent) | Delegate when | Shard as |
|---|---|---|---|
| catalog / "find all" / many urls | links → validate → `webrain_batch` | >~100 urls or mixed engines | URLs across subagents |
| specific pages (3, 4, 7) | navigate one-by-one | — (never) | — |
| infinite scroll / load more | `webrain_batch(op=interact)` per site | many interactive sites | one subagent per site-group |
| whole site | `webrain_crawl(mode=spider)` | huge / multi-subdomain | one spider subagent per subdomain (own `crawldir`) |
| discovery + extraction overlap | sequential | new site needs schema while others extract | scout subagent + extract subagents |

**Subagent self-heal (give each a fallback chain so it returns data, not
failure):**
- extraction: `webrain_observe(what=fit|flatten)` → `webrain_extract(mode=schema)`
  → `webrain_eval` → `webrain_observe(what=annotate)`
- pagination: construct `/page/N` → interact click Next → scroll → `webrain_crawl(mode=scan)`
- anti-bot: on `challenge`, **stop and report** — you re-route to the Chrome
  subagent (the subagent must not burn calls fighting a block).

**Subagent contract (give every subagent this exact scope):**
- ONE browser (`CDP_URL`), ONE task, an explicit URL list OR seed + budget
  (`max_pages`).
- Return **compact JSON only**: `{status, count, data[] | summary}`. Never raw
  HTML.
- On `challenge`/block: **report it** (don't fight it) — the orchestrator
  re-routes that URL to the Chrome subagent.

**Aggregate yourself:** dedupe by url/name (batch endpoints return sliding
windows), `webrain_extract(mode=bm25)` to keep only relevant items, then emit one answer with
a count. Subagents may nest (a subagent is just another LLM with webrain MCP)
and may use `webrain_batch concurrency` inside their own shard.

---

## 5. Login / gated flows

```
STEP 1: webrain_navigate(login_url)
STEP 2: if challenge field set → chrome way (§2), else continue
STEP 3: webrain_interact(action=type, index=<email_index>, text=email); repeat
        for the password (find field indices from navigate/observe elements, or
        webrain_observe(what=a11y))
STEP 4: webrain_interact(action=click, index=<submit_index>)
STEP 5: webrain_navigate(protected_url) → verify challenge:null + auth marker
        (e.g. "Logout", "Welcome, <user>")
STEP 6: extract (§3) → done
```

---

## 6. Anti-bot reality check (what each backend can/cannot do — verified)

| Backend | Managed JS challenge (Just a moment) | Interactive Turnstile | Non-interactive Turnstile | Screenshot / render |
|---|---|---|---|---|
| obscura v0.1.11 `--stealth` | ❌ V8 crashes | ❌ no iframe | ✅ `obscura fetch --stealth --wait 30` | ⚠️ render builds only |
| obscura v0.2.0 `--stealth` | ❌ V8 crashes (same) | ❌ no iframe | ✅ same recipe | ✅ native Rust renderer |
| real Chrome + persistent profile/session (native login) | ✅ | ✅ auto-waits; ⚠️ interactive needs human | ✅ | ✅ |
| lightpanda | ❌ | ❌ | ~ | ❌ fake placeholder PNG |

Measured on **v0.2.5** (2026-10-24): the render build paints — `obscura fetch
<url> -s out.png` wrote a 63 KB PNG — but a data-driven SPA can still stay
**unmounted**. `ab.gr/eshop/search` returns a ~1 MB shell with an empty Apollo
state (~518 B), 0 product links and 0 prices, both at the default 30 s script
deadline and with `OBSCURA_SCRIPT_DEADLINE_MS=120000`, and both through
`webrain_batch` and the bare binary (no webrain injection). Obscura's own stderr
names the cause: React hydration throws `TypeError: Cannot read properties of
undefined (reading 'firstChild')` in `_next/.../_app.js`, so the app never mounts.

The missing DOM API is **per-tag element interfaces**. Obscura exposes one class
for every element, so `HTMLTemplateElement.prototype === Element.prototype` is
`true` (Chrome: `false`). AB's `_app.js` feature-detects `HTMLTemplateElement` and
polyfills `firstChild` on it — which on obscura patches `firstChild` for **all**
elements, and `get firstChild(){ return this.content.firstChild }` throws on any
plain node. Verified both ways: obscura `sameProto: true` → `TypeError`; Chrome
`sameProto: false` → no throw, `<div>.firstChild` still works. Obscura's docs
document SVG interface ancestry ("supports interface checks in page scripts") but
never mention `HTMLElement`, `customElements` or `tagName`.

Probe before trusting any engine on a hydrating SPA — two lines, no navigation:
`HTMLTemplateElement.prototype === Element.prototype` (must be `false`) and
`document.createElement('template').constructor.name` (must be
`HTMLTemplateElement`, not `Element`). That is the documented "some Web APIs …
intentional relative to Chromium" class, not a rendering gap. **"Can paint" is
not "can boot a data-driven SPA."**

The probe is a property of the **engine build**, not of the site: it returns the
same answer for every SPA on that build. A failing probe means "this engine
cannot boot data-driven SPAs at all", so it is not a per-site verdict — do not
read it as "obscura dislikes ab.gr". Obscura v0.2.5 fails it, so ab.gr is a
Chrome case on both counts (Chrome also extracts it in ~2 s, so obscura would
never have been the only viable engine). If a later obscura build returns `false`,
the whole class is worth re-testing at once — and still verify the extracted
fields, not the engine name.

When in doubt: navigate, read the `challenge` field, choose accordingly.
webrain auto-selects the best asset for your platform + stealth preference
(render build preferred over no-render when available).

---

## 7. Task-derived lessons (verified on a full scrapingcourse.com crawl)

These come from running a real 6-source crawl (653 products) and shaving the
latency that cost an agent the most.

**Internal-link discovery is now free.** `webrain_navigate`/`webrain_observe(what=state)` return a
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
single-page `webrain_extract(mode=schema)`). Parse nothing; `text` is kept only for
backward compat. Previously the products were a JSON *string* inside `text`,
which cost repeated `data`/`text` confusion.

**Load-more / infinite-scroll shortcut (fastest path).** These pages back the
button/observer with a plain endpoint (scrapingcourse: `/ajax/products?offset=N`).
Find it by grepping the page's own scripts via `webrain_eval` for `/ajax/`.
Then `webrain_batch(op=extract, urls=[...offset=0,10,20...], base_selector,
fields)` directly — no click/scroll loop, one call. The endpoint often returns
a sliding window (offset=0→1-12, offset=10→11-22…): dedupe by url/name. This
beat the interaction loop by ~3× and avoided observer/scroll races entirely.

**SPA hydration wait.** Next.js/React/Vue sites return an HTML shell before
content renders — extracting too early yields 0 items. Before extracting, wait
for DOM growth: poll element count / text length rising with `webrain_wait`
(a JS growth expression) or `wait_selector` on the data container.
`webrain_observe(what=fit|flatten)`/`webrain_extract(mode=schema)` on an unhydrated shell is the same as
0 items (browsemind `_wait_for_js_hydration` borrow).

**`webrain_eval` + async on obscura.** Browser-level `Runtime.evaluate` does not
reliably await async JS on obscura — an async IIFE returns `null`. For async
work (fetch loops, waits), use `webrain_batch(op=interact, ...)`: its

**Stealth is automatic — no flag needed.** `stealth_js()` (navigator.webdriver=false,
UA override, chrome stub, permission query fix, Function.toString native spoof)
is injected via `Page.addScriptToEvaluateOnNewDocument` on EVERY page. This is
the CDP equivalent of `obscura --stealth`. The `--wait-until load` equivalent
is the readyState poll in `navigate_session_opts`. For `--wait 30`: use
`wait_timeout_secs=30` (default 20) with `wait_selector` + `network_idle` —
the Turnstile recipe above covers it. Summary: stealth → always on, wait-until
→ always on (readyState), wait → `wait_timeout_secs` + conditions.
interaction runs in a session where `await` resolves. Sync JS on `webrain_eval`
is fine.

**`webrain_interact(action=type)` index = the SAME list as `navigate`/`observe` elements.**
`type_text` now uses the identical selector as `ELEMENTS_JS`/`click`
(`a, button, input, select, textarea, [role="button"]`), so the index you read
from a snapshot maps 1:1 to `webrain_interact(action=type)`. Before the fix it enumerated only
`input/textarea/select` — on a page whose first interactive element is a link
(like scrapingcourse's CSRF login: `#logo-link` first), the index pointed at
the wrong field. If a type lands in the wrong box, re-check the page's FIRST
interactive element — it may be a logo link or button that shifts indices.
Hidden inputs (e.g. CSRF tokens) also occupy an index.

**Turnstile widget nuances.** Native `webrain_session(op=login)` auto-waits and
claims Turnstile/reCAPTCHA/hCaptcha widgets; a page embedding a Turnstile
*checkbox widget* (scrapingcourse `/login/cf-turnstile`) may still need a real
click on the widget's iframe checkbox (the human, or a click-capable loop).
`cf_clearance` from one challenge does not transfer to a differently-solved page.

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

---

## 8. Environment variables that change agent behaviour

Every variable here is read **once**, at process start or at CDP attach. webrain
cannot change one mid-run, and no tool call can set one: state the exact
`VAR=value`, say the MCP server must restart, and then **open a fresh tab** —
injected scripts live as long as the tab, so re-navigating the same tab keeps
the old shim and the change looks ignored.

Full table with platform defaults: [`docs/reference/environment.mdx`](reference/environment.mdx).
The same list ships inside the binary, so `webrain_guide(topic="env")` answers
on a machine with no repo checkout.

| Variable | Why an agent cares |
|---|---|
| `CDP_URL` | **The real browser choice.** `127.0.0.1:9222` = real Chrome, 9224 = obscura, 9225 = lightpanda, any remote Chrome works. Every "which engine" decision in this guide is really this variable. A raw `ws://` / `wss://` URL is accepted and passed straight through (no `/json/version` probe) — that is how Lightpanda Cloud and other hosted CDP endpoints are reached. |
| `WEBRAIN_STEALTH_NOISE=0` | The hydration fix (§2b) — drop the canvas/audio fingerprint-noise block when a page renders but never mounts. |
| `WEBRAIN_NO_STEALTH=1` | Drops *all* injected stealth JS. A diagnostic for "is our shim breaking this page?" — never present it as the fix for a Cloudflare/CAPTCHA wall. ⚠️ `webrain serp` sets it **on itself** (`main.rs:207`) — see the relaunch recipe below. |
| `WEBRAIN_NO_STATE=1` | Shared-instance mode: vault/launch/login/session answer "disabled on this instance". That is this variable, not a bug — report it. |
| `WEBRAIN_HTTP_TOKEN`, `WEBRAIN_ALLOW_UNAUTHENTICATED` | Bearer auth for `mcp --http`. An auth rejection is a deployment fact, never a page problem. |
| `WEBRAIN_PROXIES` | Rotation pool for the HTTP engines + the google/brave guest Chrome; a per-call `proxy` overrides it and never rotates. |
| `WEBRAIN_PROFILES_DIR`, `WEBRAIN_VAULT_DIR` | Where persistent profiles and the encrypted vault live. A moved profile root behaves exactly like "logged out". |
| `WEBRAIN_BROWSERS_DIR`, `WEBRAIN_CHROME`, `WEBRAIN_LIGHTPANDA`, `WEBRAIN_OBSCURA` | Engine discovery. `engine not found` = wrong path or nothing installed (`webrain install --engine …`). |
| `WEBRAIN_SKILLS_DIR` | Operator `.md` skills served through `webrain_guide(topic="skill:<name>")`. |
| `BRAVE_API_KEY`, `SERPAPI_API_KEY`, `GROQ_API_KEY`, `OPENAI_API_KEY`, `FIREWORKS_API_KEY`, `OPENROUTER_API_KEY`, `EMBED_*` | Provider tiers. Unset only means a cheaper tier or the bundled local model — never a reason to claim a scrape is impossible. |

### 8b. Relaunching the MCP server without changing its identity

A restart is **three** decisions, and getting one wrong changes the browser
identity silently. The combination that actually works is:

| Piece | Value | Why |
|---|---|---|
| `CDP_URL` | the port you validated (e.g. `http://127.0.0.1:9229`) | the server attaches at startup; with `9222` showing *no browser* it would have nothing to drive |
| `WEBRAIN_STEALTH_NOISE` | `0` only if that page needed the hydration fix | reproduces the working combination; noise-ON makes those pages hang |
| `WEBRAIN_NO_STEALTH` | **deleted from the shell** | `webrain serp` sets `WEBRAIN_NO_STEALTH=1` on itself (`main.rs:207`); if that leaks into the launching shell the server boots with **all** stealth off — no UA masking, no `webdriver` hiding — and every later result is measured against a different identity |

Intent in words: **core stealth on, fingerprint noise off, attached to the
profile you validated.** If a run "suddenly" behaves like a different browser,
check this variable before re-diagnosing the site.


