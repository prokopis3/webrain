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
| Fast scraping of non-challenged JS pages | obscura (docker, `--stealth`) — v0.2.0+ render builds screenshot/PDF; no-render builds have no paint engine; no interactive Material |
| Lightweight / minimal, want real a11y, no rendering needed | lightpanda — real AX tree, but screenshot = fake placeholder PNG |
| Static HTML, no JS/auth | `webrain_fetch_http` |

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

### Interactive SPAs (Google Flights, Material UI, calendars, search UIs)
Heavy JS apps with Material/segmented controls need **real Chrome** — obscura
has NO paint engine, so its DOM is fine but (a) `webrain_screenshot` fails and
(b) Material menus/comboboxes don't respond to synthetic clicks. Route the
batch there with `cdp_urls: ["http://127.0.0.1:9222"]` (Chrome), NOT the obscura
session backend. Recipe that works (Google Flights ATH→NYC):
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

### a11y (webrain_a11y)
Material/Google widgets are often NOT `button`: dropdowns are `combobox`,
menu items `option`, segmented controls `radio`/`tab`. If `role=X` returns [],
drop the role filter and `filter` on the label text instead (`filter` matches
name, value, and css_path).

### Extraction
Prefer `webrain_extract_json` (schema discovered via `webrain_autoschema` +
`webrain_eval`), `webrain_batch` for pagination 1..N, `webrain_spider` for
whole sites. Never guess selectors from memory — discover them.

### Auth-gated pages: cross-browser cookie transfer
Log in in **real Chrome** (Turnstile/Cloudflare auto-solve), export cookies
(`webrain_cookies` / `webrain cookies --port 9222 --out`), point the MCP at the
batch browser (`CDP_URL=...9224` obscura), `webrain_setcookies` on the session
connection, then `webrain_batch` **without** `cdp_urls` — obscura isolates
cookie jars per CDP connection, so set + batch must share one connection.
Session cookies die on Chrome restart — export on the live authenticated
browser. Full procedure: `skills/webrain/SKILL.md` → "Auth-gated pages".

Full detail: `docs/AGENT_DECISION_GUIDE.md`.
