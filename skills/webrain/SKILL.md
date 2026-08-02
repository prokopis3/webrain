---
name: webrain
version: "0.1.0"
description: Guide + scripts for driving the webrain MCP scraping tools — browser selection (real Chrome vs obscura vs lightpanda), Cloudflare/CAPTCHA/Turnstile bypass via a real-Chrome stealth sidecar, and the extraction tool matrix. Use when scraping, browser automation, or hitting anti-bot challenges with the mcp_webrain-* tools.
allowed-tools: Bash, Read
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
