---
name: webrain
version: "0.6.1"
description: Drive the webrain MCP scraping tools (mcp_webrain-*) like a pro — pick the right browser (real Chrome vs obscura vs lightpanda vs fetch_http), treat profile/session as execution state, handle anti-bot challenges natively (persistent profile + real Chrome + session via webrain_session, vault + TOTP — no Python sidecar), and choose the right extractor (autoschema → extract_json / regex / table / batch / spider). Use when scraping, browser automation, structured extraction, authenticated sites, or hitting anti-bot challenges.
argument-hint: "<what to scrape> [from <url>] [auth?] [whole site?]"
allowed-tools: Bash, Read
homepage: https://github.com/prokopis3/webrain
repository: https://github.com/prokopis3/webrain
author: prokopis3
license: MIT
user-invocable: true
---

# /webrain

You are driving the `mcp_webrain-*` MCP tools (browser scraping). This skill is
a **router** — read this page, then load only the reference or workflow your
task needs. Detail lives in `references/` and `workflows/`.

## Mandatory rules (full: `references/core-rules.md`)

1. **Browser identity, profile, and session are execution state** — never treat
   a browser as disposable.
2. **Protected navigation starts with real Chrome** + a persistent profile + a
   session — never an anonymous "blocked → fresh browser → retry" loop.
3. **No Python sidecar — challenge handling is native.** Persistent profile +
   real Chrome + session (`webrain_session(op=login)` / `webrain launch` +
   `webrain login`, vault + TOTP). Interactive CAPTCHAs the native path can't
   claim need a human in the headed browser.
4. **A challenge page is not successful navigation** — read `challenge` on EVERY
   navigate; never extract a challenge/login/consent page as target content.
5. **Verify before returning** — confirm target content exists, then report.

## Step 0 — Preflight (first use per session)

Browser/MCP down? Run `webrain doctor` (or read `webrain_guide`) once:
- MCP down → start `webrain mcp --http 9223`.
- No browser on CDP → `webrain install` then start an engine, or ask the user to
  start obscura (`docker start obscura`) / a real Chrome.

## Routing table — load only what your task needs

| Task | Load |
|---|---|
| Which browser? (engine × page-state) | `references/browser-selection.md` |
| Anti-bot challenge / gated page | `references/challenges.md` |
| Hard block / `crippled:true` / "Attention Required!" | `workflows/feed-and-news.md` §D + `references/challenges.md` §5b |
| RSS / news feeds / blocked `/rss/` endpoint | `workflows/feed-and-news.md` |
| Solve a CAPTCHA (generic tool flow: claim → scaled vision → exact geometry via `webrain_eval_in_frame` → parallel clicks → token) | `workflows/captcha-solve.md` |
| Auth / login / persistent session / cookie transfer | `references/profiles.md` |
| Structured extraction / pagination / scale | `references/extraction.md` |
| Protected / authenticated site end-to-end | `workflows/protected-site.md` |
| What NOT to do (anti-patterns) | `references/anti-patterns.md` |

## When to use

- The user asks to scrape, crawl, or extract structured data from a website.
- The user asks to navigate/interact with a browser, log into a site, or get past an anti-bot challenge.
- A page needs a real browser (SPA, Material UI, JS-rendered) and `webrain_fetch_http` is not enough.
- A batch/spider crawl over many URLs, or a whole-site sitemap.

## Recommended limits & token discipline

- **Never dump raw HTML to the model.** `webrain_get_html` / `observe(what=html)` is the LAST
  resort — use snapshot / clean / eval / the extractors for text/structure. Return summaries.
- **Batch before loop.** N pages in one `webrain_batch(op=extract, ...)`, not N sequential
  navigates. Concurrency 4-8.
- **Zero-LLM extraction first.** `autoschema` → `extract(mode=schema)` is free and deterministic;
  fall back to `eval` (JS) only when a schema can't be expressed in CSS.
- **Don't re-scrape what you have.** `webrain_snapshot` returns cached state when unchanged.
- **Filter before summarizing.** `extract(mode=bm25)` keeps only the top-k relevant items.

## How to invoke (end-to-end)

1. Parse the task: what to extract, which URL(s), auth? one page or many?
2. Preflight (first use per session).
3. `webrain_navigate(url)` → read `challenge` (null = ok) AND `links` (deduped
   same-origin hrefs for crawl/pagination discovery).
4. Branch on `challenge` (below / `references/challenges.md`).
5. Choose the browser (`references/browser-selection.md`).
6. Extract (`references/extraction.md`).
7. Report: "Extracted N items across M pages" + a sample; write bulk to `output/`.

## Challenge rule (summary — full: `references/challenges.md`)

- `challenge: null` → extract.
- `crippled: true` (block page, no challenge) → real HEADED Chrome + persistent
  profile (headless is detectable); still blocked on the same IP → clean-IP proxy
  or report.
- `challenge` set (`cloudflare_challenge`/`blocked`/`captcha`) → obscura/lightpanda
  CANNOT pass it (the challenge JS crashes). **Native fix:** persistent profile + real Chrome +
  session — `webrain_session(op=login, service, profile, url)` (vault + TOTP) or CLI
  `webrain launch <service> <profile> <url>` then `webrain login`. Or re-attach an
  already-authenticated Chrome: `webrain_session(op=open, cdp_url="http://127.0.0.1:9222")`.
- Interactive Turnstile/hCaptcha the native path can't claim → a human acts in the
  headed browser (2FA/approval gates return `waiting_for_human:true`).

## Extraction matrix (summary — full: `references/extraction.md`)

- structured list + schema known → `extract(mode=schema, base_selector, fields)`
- schema unknown → `extract(mode=autoschema)` + `eval` probe → `extract(mode=schema)`
- paginated 1..N / many URLs → `webrain_batch(op=extract, urls, base_selector, fields, concurrency=8)`
- URLs unknown → `eval`: pagination hrefs (current-path + `/<N>` + next/prev) → range → batch
- whole site → `crawl(mode=spider)`; patterns (emails/prices) → `extract(mode=regex)`
- JSON-LD → `extract(mode=jsonld)`; tables → `extract(mode=table)`
- infinite scroll / load-more → `crawl(mode=scan)` then extract; or find the `/ajax/` offset endpoint
- search → `webrain_search`; relevance → `extract(mode=bm25)`

**From-scratch discovery:** navigate → eval pagination hrefs → autoschema → eval field probe → batch → done.

## Protected sites & sessions (full: `references/profiles.md`, `workflows/protected-site.md`)

Log in ONCE on a persistent profile, then reuse that profile/session across pages —
never discard a working profile/session after a challenge. For cross-browser cookie
transfer (real Chrome → obscura batch), follow `references/profiles.md`.

## Anti-patterns (full: `references/anti-patterns.md`)

DO NOT: start protected workflows statelessly · discard a working profile/session ·
switch to a fresh browser after a challenge · treat challenge detection as success ·
assume CAPTCHA handling exists without checking runtime capability · use the stealth
sidecar as the primary path · claim an unverified bypass · extract challenge/login/
consent pages as target content · write sequential loops when `webrain_batch` exists.

## Failure modes

- `webrain_session(op=login)` returns `waiting_for_human:true` → a 2FA/approval
  gate; the human acts in the headed browser, then login again. Tell the user,
  don't loop.
- `extract(mode=schema)` returns 0 items → verify the page isn't a block page
  (check `challenge`), re-run autoschema, or wait for JS/scroll then retry.

## Security & Permissions

- Secrets live in the local vault (AES-256-GCM) and are decrypted in-process by
  `webrain_session(op=login)` — they never pass through the model. No keys are
  read or stored; no data leaves the machine except the target site requests.
