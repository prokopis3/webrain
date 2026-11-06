# Browser Selection (engine × page-state)

Pick a browser on TWO dimensions: the execution engine and the page/application
state. Never pick an engine from memory — read the `challenge` field after every
`webrain_navigate` and re-evaluate.

## State-aware decision table

| Page state | Engine | Notes |
|---|---|---|
| STATIC + PUBLIC | `webrain_scrape` / `fetch_http` (no browser) | 10-100× faster, zero memory. |
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
| guest Chrome (`--guest`) | ✅ | ✅ | ✅ | ✅ | ✅ — that is its job | ❌ one serial tab (refuses `Target.createTarget`) |
| `fetch_http` | ❌ | ❌ | ❌ | ❌ | ❌ | N/A |

## Guest Chrome is a purpose tool, not an engine

`--guest` Chrome exists for two jobs: the JS-gated SERP engines (`webrain_serp`
`engine=google|brave`) and solving an interactive CAPTCHA/Turnstile. Its profile is
the point — a persistent `serp/<profile>` unless `--fresh`, which is what keeps the
solved clearance for the next call (a fresh profile gets re-captcha'd every launch).

It cannot open a second target. Verified 2026-11-06 on Chrome 156: `PUT /json/new`
→ **500** (200 on a normal Chrome) and `Target.createTarget` → `-32000 "Failed to
open a new tab"`. A batch there runs **one serial tab** — `single_target: true`,
`effective_concurrency: 1`, 8 URLs in 5.65 s vs 1.46 s on 9222 with `concurrency: 4`
— because the batch attaches the page the browser already has instead of fanning out.

So: guest Chrome for SERP + challenges, **real Chrome / obscura / lightpanda for
`webrain_batch`, `webrain_crawl` and general navigation**. "It completes" on guest
Chrome is not a reason to route work there.

## Golden rule
Don't guess the browser. `webrain_navigate` returns `challenge` — read it, then
pick the engine for the next hop. When in doubt for a protected site, start with
real Chrome + a persistent profile.
