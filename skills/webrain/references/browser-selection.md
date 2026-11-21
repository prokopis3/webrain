# Browser Selection (engine × page-state)

Pick a browser on TWO dimensions: the execution engine and the page/application
state. Never pick an engine from memory — read the `challenge` field after every
`webrain_navigate` and re-evaluate.

## State-aware decision table

| Page state | Engine | Notes |
|---|---|---|
| STATIC + PUBLIC | `webrain_scrape` / `fetch_http` (no browser) | No render, no browser process, zero memory. |
| JAVASCRIPT + PUBLIC | obscura (or lightpanda) | Fast, parallel tabs. v0.2.0+ render builds screenshot + PDF; no interactive Material. |
| COMPLEX SPA (Material, dropdowns, calendars, segmented controls) | real Chrome (`cdp_urls:["http://127.0.0.1:9222"]`) | Needs a real layout+paint engine. Never obscura/lightpanda. |
| AUTHENTICATED (login required) | persistent profile + real Chrome + session | `webrain_session(op=login, ...)` / `webrain launch` + `webrain login`. |
| PROTECTED (Cloudflare / Turnstile / CAPTCHA / blocked) | persistent profile + real Chrome + session | Native login first; sidecar only as last resort. |
| PROTECTED + CHALLENGE (interactive Turnstile / hCaptcha) | real Chrome + human-in-the-loop | Native login auto-waits; interactive challenges need the human in the headed browser. |

## Engine matrix

| Engine | JS | SPA/Material | Screenshots | a11y | Interactive challenges | Batch |
|---|---|---|---|---|---|---|
| real Chrome | ✅ | ✅ | ✅ | ✅ | ✅ (native login) | ✅ parallel tabs |
| obscura | ✅ | ❌ (not full layout parity) | ✅ v0.2.0+ render builds (+ PDF) | partial | ❌ | ✅ parallel tabs |
| lightpanda | ✅ | ❌ | ❌ (fake placeholder PNG) | ✅ real AX tree | ❌ | sequential (single target) |
| ephemeral ("guest") Chrome | ✅ | ✅ | ✅ | ✅ | ✅ — that is its job | tabs work (`--incognito`), but serp/CAPTCHA only |
| `fetch_http` | ❌ | ❌ | ❌ | ❌ | ❌ | N/A |

## The ephemeral Chrome is a purpose tool, not an engine

It exists for two jobs: the JS-gated SERP engines (`webrain_serp`
`engine=google|brave`) and solving an interactive CAPTCHA/Turnstile. Its profile is
the point — a persistent `serp/<profile>` unless `--fresh`, which is what keeps the
solved clearance for the next call (a fresh profile gets re-captcha'd every launch).

**It used to be `--guest`, and `--guest` cannot open a tab at all.** Verified
2026-11-07 on Chrome 151 *and* 156, same launch otherwise: `Target.createTarget`
answers `-32000 "Failed to open a new tab"` (and `"Failed to open new tab - no
browser is open"` with `newWindow:false`) because Browse-as-Guest builds a window
with no Browser in Chrome's BrowserList. `--incognito` — the same ephemerality,
cookies discarded when the window closes — accepts **both** `Target.createTarget`
and `Target.createBrowserContext`, so the launcher now passes `--incognito` and a
batch there runs parallel tabs.

The remaining reason to keep bulk work off it is not a tab limit: it is a clearance
browser, so mixing a 200-URL queue into the profile that is supposed to look human
is how the clearance gets burned.


So: guest Chrome for SERP + challenges, **real Chrome / obscura / lightpanda for
`webrain_batch`, `webrain_crawl` and general navigation**. "It completes" on guest
Chrome is not a reason to route work there.

## Golden rule
Don't guess the browser. `webrain_navigate` returns `challenge` — read it, then
pick the engine for the next hop. When in doubt for a protected site, start with
real Chrome + a persistent profile.
