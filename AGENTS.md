# AGENTS.md

Guidance for AI agents working in this repo. Read before making changes or
using the `mcp_webrain-*` tools.

## Critical Rules (read first)

1. **Browser identity is state.** Profile is state. Session is state. A
   challenge is page/runtime state. Never treat a browser as disposable.
2. **Protected navigation starts with real Chrome** + a persistent profile + a
   session. Never recommend the stateless "anonymous → blocked → fresh browser →
   retry" loop.
3. **No Python in the codebase.** Challenge handling is native (vault + TOTP).
   Do not reintroduce a Python stealth sidecar for protected sites.
4. **A challenge page is not successful navigation.** Read the `challenge`
   field on every navigate; never extract a challenge/login/consent page as
   target content.
5. **Capability-driven claims only.** Never document or claim challenge
   handling the runtime doesn't provide; keep "implemented" vs "planned"
   separate.
6. **Verify before returning.** Confirm target content exists before reporting
   success.
7. **A rendered page is not a mounted page.** `navigate` returns at
   `readyState: complete`, before the SPA hydrates. Test
   `Object.keys(el).filter(k => k.startsWith('__react')).length` on a real control
   before calling a page frozen — and if a hand-opened tab of the same URL works,
   the difference is our injected stealth shim, not the site. Escape hatch:
   `WEBRAIN_STEALTH_NOISE=0` (set at attach time; needs a fresh tab). `webrain chat
   --serve` now defaults it to `0` for itself and the child `webrain mcp`, and says
   so in the page banner and on stderr — `WEBRAIN_STEALTH_NOISE=1` restores the shim
   there. Detail: `docs/AGENT_DECISION_GUIDE.md` §2b.

8. **Know which engine answered, and bridge cookies in-session.** Probe identity
   before diagnosing: `webgl:false` + UA Chrome/143 = obscura, `webgl:true` +
   Chrome/15x = real Chrome. Two `session_id`s showing identical page state are
   one browser, not a working bridge. A cookie import fills the jar of the
   connection that wrote it, so import on the session that navigates and verify
   with an authenticated endpoint (200 vs 403) - never with `set: N / readback: N`.
9. **One retry, then diagnose.** Two identical failures mean state, not luck:
   wedged engine, jar on another connection, unattached target. Wedged-obscura
   signature: `/json/version` + `/json/list` answer, `Target.getTargets` is empty,
   `Page.navigate`/`Runtime.evaluate` hang -> restart the engine with the same
   flags and re-import cookies in-session. Do not re-run the same call.

## Webrain = portable MCP scraping tool

`webrain-mcp` exposes browser/scraping tools over MCP. It is meant to be
installed on any system and driven by **any LLM** (Copilot, Claude Code, Codex,
Cursor, …). Follow `docs/AGENT_DECISION_GUIDE.md` — or just call the
`webrain_guide` MCP tool — before browser tasks.

### Browser selection (set `CDP_URL`)
| Need | Browser |
|---|---|
| Material / interactive SPA (Google Flights, dropdowns, calendars) — **never obscura/lightpanda** | real Chrome via `cdp_urls:["http://127.0.0.1:9222"]` |
| Protected / authenticated / Cloudflare / CAPTCHA / Turnstile challenges, real screenshots, rendering | **real Chrome + persistent profile + session** (`webrain launch` / `webrain login`) |
| **`webrain_serp` google/brave, or an interactive CAPTCHA/Turnstile to solve** | **the ephemeral ("guest") Chrome** — `--incognito` since 2026-11-07; `--guest` cannot open a tab (`-32000 "Failed to open new tab - no browser is open"`, Chrome 151 + 156). A *purpose tool* with exactly two callers: `webrain_serp` and the CAPTCHA solve. Its profile (`serp/<profile>`, persisted unless `--fresh`) carries the clearance to the next call |
| Fast scraping of non-challenged JS pages | obscura (docker, `--stealth`) — v0.2.7+ mounts data-driven SPAs natively (v0.2.6/0.2.7 fixed the HTML interface prototypes); v0.2.5 and older cannot. Batch = one tab per CDP **connection**, K≈4 (§ Obscura batch) |
| Lightweight / minimal, want real a11y, no rendering needed | lightpanda — real AX tree, but screenshot = fake placeholder PNG |
| Static HTML, no JS/auth | `webrain_fetch_http` |

### Obscura batch: tabs share the process, connections add CPU (measured)

Tabs on one obscura connection **run concurrently but share one process's CPU**
— they do not serialize. Measured on 50 light recipe pages (~0.5 s each):
`concurrency: 6` on one connection = **10.5 s** vs **23.9 s serial** (2.3×);
K=10 = 10.1 s but each page +57% slower. Heavy ~1 MB pages saturate that one
process, so there the recipe is one connection per page, K≈4; heavy origins
also throttle in-flight loads. Measured on a heavy e-commerce product page
(1.07 MB Next.js SPA, obscura v0.2.5, 16 cores):

| in-flight | layout | wall | per-row | throughput |
|---|---|---|---|---|
| 1 | 1 conn | 3.2 s | 3.2 s | 0.31 /s |
| **4** | **4 conns, 1 proc** | **7.4 s** | **7.0–7.3 s** | **0.54 /s (peak)** |
| 8 | 8 conns, 1 proc | 20.1 s | 16.0–19.9 s | 0.40 /s |
| 8 | 8 procs × 1 tab | 20.5 s | 17.1–20.4 s | 0.39 /s |
| 10 | 10 conns, 1 proc | 30.2 s | 26.1–29.0 s | 0.33 /s |
| 20 | 1 proc × 20 tabs | 30.5 s | **0/20** (30 s cap) | — |
| 20 | 10 procs × 2 tabs | 42.1 s | 13/20; retry 7/7 in 12.4 s → **54.5 s** | — |

Recipe:

```json
{"op": "eval", "urls": ["..."],
 "cdp_urls": ["http://127.0.0.1:9224", "http://127.0.0.1:9224",
              "http://127.0.0.1:9224", "http://127.0.0.1:9224"],
 "per_backend_concurrency": 1,
 "disable_resources": true, "block_trackers": true}
```

- Repeat the **same endpoint** in `cdp_urls` (one connection per entry) and force
  `per_backend_concurrency: 1` — it defaults to `concurrency`, which stacks tabs
  back onto one connection.
- K≈4 for ~1 MB JS-heavy pages; K=6 was slower (52.4 s / 20 pages on v0.2.7) and
  K=20 returned 0/20. Light pages scale fine (8 example.com pages = 834 ms).
- Only one obscura process is needed (`--max-connections` default 128);
  `--workers N` is for isolation, not speed (1 proc × 8 conns = 8 procs × 1 conn).
- Retry a missed `Page.navigate` instead of lowering K for everyone:
  `retry: 1, retry_on: "timeout"` re-navigates the SAME tab once (the row then
  reports `retries: 1`). `navigate_timeout_ms` (5 000–120 000) raises the fixed
  30 s command cap when an origin legitimately needs longer; `delay_ms` +
  `autothrottle` pace a rate-limited origin. Batch stats expose
  `retries_used`, `connect_errors`, `suggested_concurrency`, `autothrottled`.
- v0.2.7 numbers: 20 heavy product pages at K=4 → **20/20 in 43.3 s**.

Full recipe, version gate and consent handling: `skills/webrain/references/obscura-batch.md`.

### Engine install (agent-browser style)
Mirrors vercel-labs/agent-browser: `webrain install` downloads **Chrome for
Testing** into a cache dir (`WEBRAIN_BROWSERS_DIR`/platform dir) and that build
then wins discovery over system Chrome. `webrain install --engine obscura`
downloads the latest **Obscura** release (GitHub, `--stealth` picks the
BoringSSL build). `webrain lightpanda [--port N]` / `webrain obscura [--port N]`
spawn the lightpanda/obscura CDP servers (binaries from PATH, `~/.lightpanda`,
`~/.obscura`, `~/.local/bin`, or `WEBRAIN_LIGHTPANDA`/`WEBRAIN_OBSCURA`). Like
agent-browser's `--engine chrome|lightpanda`, all engines speak CDP —
`CdpBackend` connects to any of them.

### Challenge rule (read `challenge` on every navigate)
`webrain_navigate` returns `challenge`. If non-null (`cloudflare_challenge` /
`blocked` / `captcha`), the page is gated. The fix is real Chrome with a
**persistent profile + session**: `webrain launch <service> <profile> <url>`
then `webrain login <service> <profile>` (vault + TOTP), or re-attach an
already-authenticated Chrome by pointing `CDP_URL` at its port. obscura/
lightpanda cannot pass interactive challenges. No Python sidecar — interactive
CAPTCHAs need a human in the headed browser.

### Identity coherence (proxy exit vs. what the page reads)
Chrome reports the **host's** timezone and locale. Behind a foreign proxy that
leaves the page telling two conflicting stories — the incoherence camoufox's
`coherence.py` throws out. Set `WEBRAIN_TZ` (e.g. `America/Chicago`) and
`WEBRAIN_LOCALE` (e.g. `en-US`) to the exit IP's values before starting the
runtime; both are applied as CDP emulation (`Emulation.setTimezoneOverride` /
`setLocaleOverride`) on every attached tab, before page scripts run. Geolocation
is not set: without coordinates it would be a third lie, not a fourth truth.

### Interactive SPAs (Google Flights, Material UI, calendars, search UIs)
Heavy JS apps with Material/segmented controls need **real Chrome** for
interaction. Obscura's render build **does** paint (`obscura fetch <url> -s
out.png` writes a real PNG), and since **v0.2.7 it also mounts data-driven SPAs**
— but Material menus/comboboxes still don't respond to synthetic clicks.

Version gate (this is a property of the engine build, not the site). On obscura
**≤ v0.2.5** a data-driven SPA stays **unmounted**: measured on a large
supermarket SPA search page, v0.2.5 — even with
`OBSCURA_SCRIPT_DEADLINE_MS=120000` — returns a ~1 MB shell whose Apollo cache
is empty (~518 B), with 0 product links, 0 prices and no consent modal.
Obscura's own stderr names the cause: React hydration throws `TypeError: Cannot
read properties of undefined (reading 'firstChild')` inside `_next/.../_app.js`.
Root cause is a missing DOM API: obscura v0.2.5 has **one** `Element` prototype
for every tag, so `HTMLTemplateElement.prototype === Element.prototype`, and the
site's `HTMLTemplateElement.firstChild` polyfill therefore breaks `firstChild`
on all elements. v0.2.6 fixed 28 missing HTML interface prototypes (`#1387`) and
v0.2.7 fixed 24 more aliased interfaces (`#1482`), plus Windows runtime
packaging (`#1492`).

On **v0.2.7 the same page mounts natively**: 20 product cards, the full 537-item
result count, prices, and the cookie-consent modal. Accept consent with a click
on its accept button (it writes `VersionedCookieConsent`); the v0.2.5-era
`HTMLTemplateElement` init script polyfill is **obsolete on v0.2.7 — remove
it**. Upgrade with `webrain install --engine obscura --stealth --force` (a plain
install says "already installed" and does nothing).

Probe the engine build in two lines before trusting it on an SPA:
`HTMLTemplateElement.prototype !== Element.prototype` must be `true`, and
`document.createElement('template').constructor.name` must be
`HTMLTemplateElement`. A failing probe means "this build cannot boot data-driven
SPAs at all" — upgrade, don't write per-site polyfills.

Route *interactive* Material work (dropdowns, calendars, segmented controls) to
real Chrome with `cdp_urls: ["http://127.0.0.1:9222"]`; data extraction from
SPA pages on obscura v0.2.7+ is fine. Recipe that works for Google Flights
ATH→NYC on Chrome:
1. Google consent gate: set cookies `SOCS` + `CONSENT` on `.google.com` via
   `webrain setcookies tmp/google_consent_cookies.json --port 9222` once, then
   every tab skips the dialog.
2. `webrain_batch` op=interact, `cdp_urls:[...9222]`, one URL per date. The
   interaction opens the Material dropdown (`div.VfPpkd-TkwUic` text "Round
   trip"), clicks `[role="option"]` "One way", waits for results, reads the
   result CARDS (`li.pIav2d`).
3. Read prices from result cards/containers, NEVER from bare `$` text nodes —
   Google renders a date-grid of prices that pollutes raw-text scrapes with
   wrong values (a $533 ghost vs the real $715 card).
4. Keep batch interactions short: background tabs throttle `setInterval` to
   ~1/s and long `await` promises can resolve as null. Prefer sync DOM work +
   one awaited `fetch`; avoid long polling loops.

### a11y (`webrain_observe` with `what=a11y`)
Material/Google widgets are often NOT `button`: dropdowns are `combobox`,
menu items `option`, segmented controls `radio`/`tab`. If `role=X` returns [],
drop the role filter and `filter` on the label text instead (`filter` matches
name, value, and css_path).

### Waits (`wait_selector`)
The state is satisfied by **any** match (fixed in this build); before the fix only
the first DOM match counted, and an SPA list renders the cards twice (hidden
mobile container first), so `visible` never arrived. Measured on a 27-URL
grocery search list, `concurrency: 6`: `visible` + 30 s = **159 s** (810 s of
`ms_total` was pure timeout), `attached` + 6 s = **10 s**, identical rows.
- Use `wait_selector_state: "attached"` with `wait_timeout_secs: 6` on lists.
- Anchor on the repeated card (`div.product`), not on a shell element that exists
  before the cards (`#ProductsClass` answered in 0.2-2.4 s with `found: 0`).
- A "no results" page never matches the card selector, so it pays the full
  timeout — keep it short.

### Extraction
Prefer `webrain_extract` (`mode=json` with a schema discovered by
`mode=autoschema` plus `webrain_eval`), `webrain_batch` for pagination 1..N,
`webrain_crawl` (`mode=spider`) for whole sites. Never guess selectors from
memory — discover them.

### Auth-gated pages: cross-browser cookie transfer
Log in in **real Chrome** (Turnstile/Cloudflare auto-solve), export cookies
(`webrain_session` with `op=cookies`, or the CLI `webrain cookies --port 9222 --out`),
point the MCP at the batch browser (`CDP_URL=...9224` obscura), then
`webrain_session` with `op=setcookies` on the session connection, then
`webrain_batch` **without** `cdp_urls` — obscura isolates
cookie jars per CDP connection, so set + batch must share one connection.
Session cookies die on Chrome restart — export on the live authenticated
browser. Full procedure: `skills/webrain/SKILL.md` → "Auth-gated pages".

**Cloudflare clearance CAN be transferred to obscura stealth (verified
2026-11-25).** The old "cf_clearance never transfers" rule
is too absolute: with the obscura **v0.2.7 stealth build** (BoringSSL TLS
impersonation) started with a **UA matching the Chrome that minted the cookie**
(`--user-agent`), and the same egress IP, Cloudflare accepts an imported
`cf_clearance` and serves the origin page to obscura. Procedure:

1. Headed Chrome solves the challenge (headless Chrome and obscura are both
   detected: `crippled: true` / "Just a moment…").
2. `webrain_session{op:"cookies", session_id:"<chrome-session>"}` → export,
   keep only the target-domain cookies including `cf_clearance`.
3. `webrain_session{op:"setcookies", cookies:[...]}` **without** `session_id`
   — that writes the MCP's default connection (the one `webrain_batch` uses
   when `cdp_urls` is omitted).
4. `webrain_batch` **without** `cdp_urls` (same connection). New connections do
   NOT inherit the clearance: duplicating the endpoint in `cdp_urls` gets
   challenged again, and a `--storage-dir` profile did not carry it across
   connections in testing. The cleared batch stays on one connection, but its
   tabs run concurrently: 20 recipes = 4.6–9.7 s, and 50 recipe pages at
   K=6 = **10.5 s wall** (23.9 s serial), all `challenge:false`.
5. Faster variant when the clearance holder is Chrome: batch **on Chrome**
   (`cdp_urls:["http://127.0.0.1:<chrome-port>"]`, concurrency 6) — Chrome
   shares the profile's cookies across tabs: 20 recipes in 4.6 s.

If the import still shows "Just a moment…", the minting Chrome's UA/TLS/IP does
not match the obscura process: relaunch obscura with that exact `--user-agent`
and the same exit, re-export, repeat once — otherwise keep the batch on Chrome,
which is the supported path.

Full detail: `docs/AGENT_DECISION_GUIDE.md`.
