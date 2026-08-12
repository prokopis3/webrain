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
| `fetch_http` | ❌ | ❌ | ❌ | ❌ | ❌ | N/A |

## Golden rule
Don't guess the browser. `webrain_navigate` returns `challenge` — read it, then
pick the engine for the next hop. When in doubt for a protected site, start with
real Chrome + a persistent profile.
