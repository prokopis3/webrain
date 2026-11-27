# Obscura batch: real concurrency, measured

How to get maximum throughput from an obscura engine with `webrain_batch`. The
short version: **one tab per CDP connection, ~4 in flight on heavy pages, and
upgrade to v0.2.7+**. Everything below was measured on this repo (2026-11-25,
obscura v0.2.5 and v0.2.7, Windows, 16 cores, a 1.07 MB Next.js product page,
plus example.com as a light control).

## 1. Engine version gate (do this first)

```bat
obscura --version
webrain install --engine obscura --stealth --force
```

- `webrain install --engine obscura` **does nothing when an engine is already
  installed** — it prints "Obscura already installed" and exits 0. Use
  `--force` to actually fetch the latest release.
- v0.2.5 (2026-10-17) fails the interface probe below and cannot mount data-driven
  SPAs. v0.2.6 (2026-10-31) fixed **28 missing HTML interface prototypes**
  (`#1387`); v0.2.7 (2026-11-13) fixed 24 more aliased interfaces (`#1482`) and
  Windows runtime packaging (`#1492`).
- Result: on **v0.2.7 a large supermarket SPA search page mounts natively** —
  the full result count, a rendered cookie-consent modal, and prices — with
  **no polyfill**. The `HTMLTemplateElement` init script from the v0.2.5 era is
  obsolete on v0.2.7.
- v0.2.7 also gives each CDP connection an isolated copy-on-write localStorage
  snapshot and a fresh IndexedDB store (`#1478`), and does not block CDP while an
  awaited evaluation settles (`#1310`).

Probe the engine build (property of the binary, same answer for every SPA):

```js
HTMLTemplateElement.prototype !== Element.prototype          // must be true
document.createElement('template').constructor.name          // must be 'HTMLTemplateElement'
```

If either is wrong, upgrade — do not build per-site polyfills on v0.2.5.

## 2. The batching rule: connections, not tabs

`webrain_batch` without `cdp_urls` opens `concurrency` **tabs on ONE connection**
(`Target.createTarget` per URL). Tabs on one connection **do run concurrently** —
they share one obscura process's CPU and one WebSocket, they do NOT serialize
one-by-one. Measure it on light pages: 50 recipe pages (~0.5 s each) went
**10.5 s at K=6 on one connection** vs **23.9 s serial (K=1)** = 2.3× — real
parallelism. The limit is CPU: six tabs make each page ~2.5× slower, so the
speedup is sublinear.

Heavy pages (~1 MB HTML + a large JS bundle) saturate that one process: 8 tabs
on one connection took 20.1 s for 8 pages vs 7.4 s for 4 connections × 1 tab —
one process cannot render four 1 MB pages at once. So the rule splits by page
weight:

- **Light pages** (~0.3–0.6 s render): one connection with `concurrency: 6` is
  the sweet spot (10.5 s / 50 pages); K=10 buys 0.4 s and costs +57% per page.
- **Heavy pages** (~3 s render): give each page its **own connection** and keep
  K≈4 (below).

To get true per-page isolation on heavy pages, give each page its **own
connection**:

```json
{
  "op": "eval",
  "urls": ["https://site/p/1", "https://site/p/2", "..."],
  "cdp_urls": ["http://127.0.0.1:9224", "http://127.0.0.1:9224",
               "http://127.0.0.1:9224", "http://127.0.0.1:9224"],
  "per_backend_concurrency": 1,
  "disable_resources": true,
  "block_trackers": true
}
```

- Repeat the **same endpoint** in `cdp_urls`; each entry is a separate WebSocket
  connection (separate thread), and URLs are round-robin'ed across them.
- Set `per_backend_concurrency: 1`. It defaults to `concurrency` (and the code
  clamps it to `tab_budget / len(cdp_urls)`), so leaving it out can put several
  tabs back on one connection — the serial trap.
- One obscura **process** serves many connections (`--max-connections`, default
  128); you do not need one process per connection. `--workers N` spawns N
  worker processes (reliability / isolation), not extra speed: measured 1
  process × 8 conns = 20.1 s vs 8 processes × 1 conn = 20.5 s for the same 8
  pages.

## 3. Pick K by measurement — it is origin-bound, not engine-bound

Light pages show the engine parallelizes fine: 8 example.com pages on one
process, 8 connections = **834 ms wall** (~0.1 s/page).

Heavy pages are limited by the origin (an Akamai-fronted site; each page is
~1 MB HTML plus a large Next.js bundle). Measured, obscura v0.2.5:

| in-flight pages | layout | wall | per-row ms | throughput |
|---|---|---|---|---|
| 1 | 1 conn | 3.2 s | 3 200 | 0.31 /s |
| **4** | **4 conns, 1 proc** | **7.4 s** | **7 049–7 315** | **0.54 /s** |
| 8 | 8 conns, 1 proc | 20.1 s | 15 994–19 946 | 0.40 /s |
| 8 | 8 procs × 1 tab | 20.5 s | 17 090–20 425 | 0.39 /s |
| 10 | 10 conns, 1 proc | 30.2 s | 26 065–28 979 | 0.33 /s |
| 20 | 1 proc × 20 tabs | 30.5 s | **0/20** (30 s `Page.navigate` cap) | — |
| 20 | 10 procs × 2 tabs | 42.1 s | 13/20 ok, 16 915–39 716 | + retry 12.4 s → 20/20 in 54.5 s |

Throughput peaks around **4 in-flight pages**; more concurrency makes every page
slower and pushes rows into the fixed 30 s `Page.navigate` ceiling. Version
v0.2.7, 20 heavy product pages with every row successful:

| K (connections) | wall | result |
|---|---|---|
| 4 | **43.3 s** | 20/20, 4 960–12 769 ms / row |
| 6 | 52.4 s | 20/20, 4 947–19 816 ms / row |
| 20 | 30.4 s | **0/20**, all timed out |

So: **start at K = 4, raise only while per-row ms stays flat, and add
`retry: 1, retry_on: "timeout"` so the 30 s misses recover in the same call**
instead of a manual second pass. 20 heavy pages ≈ 40–45 s; a burst of 20
in-flight is strictly worse (0 rows; with `retry` they only spend another
cap-length attempt), so recover misses with `retry` rather than raising K. The
same K rule applies to `webrain_crawl(mode=spider)` concurrency.

## 4. Speed knobs that work

| Knob | Why |
|---|---|
| `disable_resources: true` | blocks fonts/images/media on every tab |
| `block_trackers: true` | drops the tracker list before it signs anything |
| no `network_idle` | obscura waits for the load event; idle waits add seconds |
| `wait_selector_state: "attached"`, `wait_timeout_secs: 6` | only when the data needs the DOM; a miss costs the full timeout |
| `op=eval` reading `__NEXT_DATA__` / JSON-LD | zero-LLM, no CSS selectors; the JSON parse was 8–57 ms per page |
| `retry: 1` + `retry_on: "timeout"` | re-navigates the SAME tab once after a 30 s miss; the row reports `retries: 1`, stats `retries_used` |
| `navigate_timeout_ms: 45000` | raises the fixed 30 s `Page.navigate` cap (5 000–120 000 ms) when the origin legitimately needs longer; `wait_timeout_secs` above 25 only works with it |
| `delay_ms` + `autothrottle` + `autothrottle_max_ms` | pace a rate-limited origin: fixed pause per URL, grown 200 ms → max after failures and decayed on success; `stats.autothrottled` says whether it fired |
| watch `stats.suggested_concurrency` / `connect_errors` | a refused cloud connection is no longer a fake result row; the report tells you the K the surviving backends can sustain |

## 5. Cookie consent (v0.2.7 mounts the CMP)

On v0.2.5 the consent component never mounted. On v0.2.7 it renders, and on
consent-gated SPAs it covers the content until accepted. Accept it once per
session/profile with a real click (React listens for trusted-ish events; a
`button.click()` on the accept button worked here and writes
`VersionedCookieConsent`):

```json
{
  "op": "interact",
  "urls": ["https://example.com/list"],
  "interaction": "(async () => { const b=[...document.querySelectorAll('button')].find(x=>/^(accept|agree|i agree|allow all)$/i.test((x.innerText||'').trim())); if (b) b.click(); await new Promise(r=>setTimeout(r,800)); return 'ok'; })()",
  "base_selector": "li.product-item",
  "fields": [{"name": "text", "selector": "", "type": "text"}]
}
```

Batch connections get isolated storage on v0.2.7, so consent can re-appear per
connection. It does **not** block `__NEXT_DATA__` extraction (server-rendered
payload is already in the document), only DOM/visual extraction — use the
`op=interact` form above when you need the rendered cards.

## 5b. Cloudflare-gated sites: Chrome-solved clearance → obscura (verified)

Cold obscura and headless Chrome are both detected (`crippled: true`, "Just a
moment…"); **headed** real Chrome passes. The clearance can then be moved to
obscura stealth, so the batch runs on obscura:

1. Start obscura **v0.2.7+ `--stealth`** with a UA that matches the minting
   Chrome, e.g.
   `obscura serve --port 9222 --stealth --user-agent "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/158.0.0.0 Safari/537.36"`.
2. Solve the challenge in headed Chrome (`webrain launch <svc> <profile> <url> --port 9235`),
   open a session on it, and export cookies:
   `webrain_session{op:"cookies", session_id:"<chrome-session>"}`.
3. Keep the target-domain cookies (notably `cf_clearance`) and import them into
   the **default** connection — no `session_id`:
   `webrain_session{op:"setcookies", cookies:[...]}`.
4. Run `webrain_batch` **without `cdp_urls`** so it uses that same connection.

Measured (2026-11-25): a 20-recipe batch on obscura went
**20/20, `challenge:false`, wall 9.7 s** (311–832 ms/row). The same batch on the
Chrome that held the clearance was **4.6 s at `concurrency: 6`** — Chrome shares
one profile's cookies across tabs. The cleared obscura connection's tabs run
concurrently too, just on one process's CPU: 50 recipe pages at K=6 on that
connection = **10.5 s** (23.9 s serial).

Limits that matter:

- New obscura connections get a fresh cookie jar: duplicating the endpoint in
  `cdp_urls` (for K-way parallelism) is challenged again, and a `--storage-dir`
  profile did **not** carry the clearance across connections in testing. A
  cleared obscura batch therefore stays on the one cleared connection — but runs
  K tabs on it concurrently (K≈6 for light pages).
- The transfer only works when the UA/TLS/IP story matches the Chrome that
  minted the cookie. If it still shows "Just a moment…", re-check the UA
  (`--user-agent`), the exit IP, and the cookie domain; otherwise batch on
  Chrome — the supported path.
- `challenge` is `null` on the Cloudflare interstitial too (it is a
  `crippled:true` block page). Judge success by the **title** (not
  "Just a moment…") and by extracted fields.

## 6. Verification checklist

1. `obscura --version` ≥ 0.2.7 **and** the two-line interface probe passes.
2. `webrain_navigate` the target → `challenge: null`, correct title, and the
   expected content present (not a consent/login shell).
3. `webrain_batch` with duplicated `cdp_urls` + `per_backend_concurrency: 1`;
   check `stats.effective_concurrency` equals the connection count and that
   `ms_wall` is well under `ms_total` (parallel, not serial).
4. If any row says `Page.navigate timed out after 30s`, retry just those rows —
   do not raise K.
