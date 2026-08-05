---
name: webrain
version: "0.2.0"
description: Drive the webrain MCP scraping tools (mcp_webrain-*) like a pro — pick the right browser (real Chrome vs obscura vs lightpanda vs fetch_http), get past Cloudflare/CAPTCHA/Turnstile via a real-Chrome stealth sidecar, and choose the right extractor (autoschema → extract_json / regex / table / batch / spider). Use when scraping, browser automation, structured extraction, or hitting anti-bot challenges.
argument-hint: "<what to scrape> [from <url>] [auth?] [whole site?]"
allowed-tools: Bash, Read
homepage: https://github.com/prokopis3/webrain
repository: https://github.com/prokopis3/webrain
author: prokopis3
license: MIT
user-invocable: true
---

# /webrain

You are driving the `mcp_webrain-*` MCP tools (browser scraping). This skill
tells you which browser to use, how to get past anti-bot challenges, and which
extraction tool fits — plus the real-Chrome sidecar script that actually solves
Cloudflare challenges.

## Resolve `SKILL_DIR` (before any command)

`SKILL_DIR` = the absolute path of the directory containing the `SKILL.md` you
just Read. Scripts are always `${SKILL_DIR}/scripts/...`:

```
Read <host>/skills/webrain/SKILL.md  →  SKILL_DIR=<host>/skills/webrain
```

Guard once: `[ -f "$SKILL_DIR/scripts/preflight.py" ]` or `python` (Windows).

## Step 0 — Preflight (first use per session)

```bash
python "${SKILL_DIR}/scripts/preflight.py"      # JSON: mcp_up, cdp[], recommend
```

- `mcp_up: true` → the webrain MCP server is reachable (127.0.0.1:9223).
- `cdp[]` lists live browser backends; `recommend` = `real-chrome` | `obscura` | `none`.
- `--check` exits 0 when mcp + a browser are both up (silent on success).

Branch: browser down → ask the user to start obscura (`docker start obscura`) or
a real Chrome; MCP down → start `webrain.exe mcp --http 9223`.

## When to use

- The user asks to scrape, crawl, or extract structured data from a website.
- The user asks to navigate/interact with a browser, log into a site, or get past an anti-bot challenge.
- A page needs a real browser (SPA, Material UI, JS-rendered) and `webrain_fetch_http` is not enough.
- A batch/spider crawl over many URLs, or a whole-site sitemap.

## Recommended limits & token discipline

- **Never dump raw HTML to the model.** `webrain_get_html` is the LAST resort - use
  `webrain_snapshot` / `webrain_clean` / `webrain_eval` / the extractors for text/structure.
  Return summaries, not page blobs.
- **Batch before loop.** N pages 1..N in one `webrain_batch(op=extract, ...)` call, not N
  sequential `webrain_navigate` calls. Batch concurrency 4-8.
- **Zero-LLM extraction first.** `webrain_autoschema` → `webrain_extract_json` is free and
  deterministic; only fall back to `webrain_eval` (JS) when a schema can't be expressed in CSS.
- **Don't re-scrape what you have.** If the page state is unchanged, `webrain_snapshot` returns
  the cached state and saves tokens.
- **Filter before summarizing.** `webrain_bm25` keeps only the top-k relevant items.

## How to invoke (end-to-end)

**Step 1 - parse the task.** Separate: what to extract, from which URL(s), whether auth is
needed, and whether it's one page or many. Example: `scrape all product titles + prices from
https://example.com/products` → extract = title+price, urls = 1 page (probe for pagination), no auth.

**Step 2 - preflight (first use per session).** Run `preflight.py`; if MCP or a browser is down,
fix before scraping (start `webrain mcp --http 9223` or a Chrome/obscura).

**Step 3 - navigate the seed.** `webrain_navigate(url)` → read `challenge` (null = ok) AND `links`
(deduped same-origin links for crawl/pagination discovery in one call).

**Step 4 - branch on the challenge field (EVERY navigate):**
- `challenge: null` → proceed to extract.
- `challenge: cloudflare_challenge|blocked|captcha` → real Chrome + `stealth_solve.py` (below).
  obscura/lightpanda CANNOT pass interactive challenges.

**Step 5 - choose the browser** (per the decision guide): real Chrome for Material/SPA/challenge,
obscura for parallel batch, lightpanda for real a11y, `fetch_http` for static HTML.

**Step 6 - extract** (per the extraction matrix): autoschema → extract_json for structured lists,
regex/table/jsonld for patterns/tables/metadata, batch/spider for scale, eval for custom JS.

**Step 7 - report.** Summarize: "Extracted N items across M pages" with a sample, not raw blobs.
Write bulk results to `output/` JSON when the user wants a file.

## Decision guide (condensed)

**Browser selection:**
| Situation | Browser |
|---|---|
| Cloudflare / Turnstile / CAPTCHA / interactive challenge, screenshots, rendering | **real Chrome** (sidecar below) |
| JS-rendered page, no challenge, batch scraping | **obscura** (docker, `--stealth`) |
| Static HTML, no JS/auth | `webrain_fetch_http` (no browser) |

**Challenge rule (read `challenge` on EVERY `webrain_navigate`):**
- `challenge: null` → extract.
- `challenge` set (`cloudflare_challenge`/`blocked`/`captcha`) → obscura/lightpanda
  CANNOT pass it (no paint engine). Use the **chrome way**:

```bash
python "${SKILL_DIR}/scripts/stealth_solve.py" <login_or_challenge_url> --cdp-port 9222 --headed
```

It waits out the challenge, logs in (`--creds user:pass` or the page's demo
creds), exports cookies, keeps Chrome alive on 9222. Then point the webrain MCP
at `CDP_URL=http://127.0.0.1:9222` and re-navigate — the session is shared.

**Extraction matrix:**
- structured list + schema known → `webrain_extract_json(base_selector, fields)`
- schema unknown → `webrain_autoschema` (container) + `webrain_eval` (field probe) → extract_json
- paginated 1..N / many URLs → `webrain_batch(op=extract, urls, base_selector, fields, concurrency=8)`
- URLs unknown → `webrain_eval`: pagination hrefs = current-path + `/<N>` + next/prev labels (no hardcoded classes) → range → batch
- whole site → `webrain_spider`; patterns (emails/prices) → `webrain_extract_regex`
- JSON-LD → `webrain_get_jsonld`; tables → `webrain_table`
- infinite scroll / load-more → `webrain_scan` then extract, or `webrain_click` loop
- search → `webrain_search`; relevance → `webrain_bm25`

**From-scratch discovery (schema + URLs unknown):** navigate → eval pagination hrefs → autoschema → eval field probe → batch → done(summary="Extracted N items across M pages").

## Auth-gated pages: cross-browser cookie transfer (LLM MUST follow this)

When a page needs a login / Turnstile / Cloudflare session (auth-gated data) and
you want to batch it through **obscura** (fast, cheap) instead of keeping Chrome
open: log in once in real Chrome, **export the session cookies, import them into
the MCP session connection, then batch on that same connection.**

Proven sequence (scrapingcourse Turnstile case):

1. **Log in in real Chrome** — `webrain_launch(service, profile, login_url)` then
   `webrain_login` (or CLI `webrain login`), or the human logs in on the headed
   Chrome. Turnstile auto-solves in real Chrome (`cf-turnstile-response` token
   appears; obscura CANNOT solve it — `token` stays 0).
   Verify: navigate to the protected page → expect "Welcome/Logout", not the
   login form.
2. **Export cookies on the LIVE authenticated browser** — `webrain_cookies`
   (MCP, session backend) or CLI `webrain cookies --port 9222 --out file.json`.
   ⚠️ Session cookies (`scrapingcoursecom_session`, `PHPSESSID`, `laravel_session`,
   etc.) **do NOT survive a Chrome restart** — if you restarted Chrome you MUST
   re-login first. Export while Chrome is alive.
3. **Point the MCP at the target browser** — restart `webrain.exe mcp --http 9223`
   with `$env:CDP_URL="http://127.0.0.1:9224"` (obscura). Do NOT use the CLI
   `setcookies` here: the CLI opens a fresh connection each run.
4. **Import on the MCP session connection** — `webrain_setcookies(cookies=...)`
   (same call as step 2's output). Readback should be 12 ≈ export count.
5. **Batch WITHOUT `cdp_urls`** — `webrain_batch(op=..., urls, ...)` with no
   `cdp_urls` uses the MCP's persistent session connection, so it shares the
   imported cookies. ⚠️ obscura (stealth) isolates cookie jars **per CDP
   connection**: any fresh connection (`cdp_urls`, CLI, separate tool call that
   reconnects) starts with EMPTY cookies. Set + batch MUST share one connection.
6. Write results to `output/` JSON.

Gotchas learned the hard way:
- `Network.getCookies` is deprecated on Chrome 151 and returns `[]` — the
  backend now prefers `Storage.getCookies` (browser-level, all context cookies
  incl. HttpOnly). If `webrain cookies` ever shows 0 on a logged-in browser,
  it's a restart-killed session cookie, not the tool.
- The auth cookie is often HttpOnly (invisible to `document.cookie`) — always
  verify via `webrain_cookies`/`webrain navigate` text, not `eval(document.cookie)`.

## Failure modes
- `stealth_solve.py` prints `challenge_cleared=False` → still gated; the challenge
  may be interactive Turnstile needing a human click or a solver service — tell
  the user, don't loop.
- `preflight.py --check` non-zero → missing MCP or browser; fix per Step 0.
- `webrain_extract_json` returns 0 items → verify the page isn't a block page
  (check `challenge`), re-run autoschema, or wait for JS/scroll then retry.

## Security & Permissions
- `stealth_solve.py` launches real Chrome locally with a temp profile; cookies
  written to `--out` are the site's own session cookies (cf_clearance etc.).
- No keys are read or stored; no data leaves the machine except the target site
  requests. Review `scripts/` before first use.

## Bundled scripts
- `scripts/preflight.py` — MCP/CDP status (`--check` silent, JSON default).
- `scripts/stealth_solve.py` — real-Chrome Cloudflare/CAPTCHA bypass + login + cookie export.
- `scripts/build-skill.sh` — package `dist/webrain.skill` for claude.ai upload (dev).
