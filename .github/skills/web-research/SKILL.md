---
name: web-research
version: "1.0.0"
description: Turn any website (or set of websites) into structured, verifiable data with the webrain MCP tools — discover a schema once, batch it across many URLs, persist the payload to disk, and report with source URLs. Use when researching a question across pages/sites, building a dataset from web pages, monitoring a site, or comparing what several sites say. For shopping / prices / product specs use `product-market-research` instead.
argument-hint: "<question or what to extract> [from <url|site>] [how many pages?]"
allowed-tools: Bash, Read, Write
---

# /web-research

You are converting web pages into a dataset, then answering from it. The whole
skill is one loop: **discover → batch → persist → verify → report**.

## Rule 0 — the loop is not "navigate and look"

`webrain_navigate` is for **one** page you need to understand: a schema to
discover, a wall to inspect, a form to open. It is never how you read 10+ pages.
Everything at scale is `webrain_batch` with **one** JS extractor.

## The loop

### 1. Preflight (once per session)
```
webrain doctor            # engines, CDP ports, vault
webrain mcp --http 9223   # the MCP server, if down
```
Pick the engine before anything else:

| Page | Engine |
|---|---|
| **Anything you can GET** — try this FIRST | `webrain_scrape` with `max_chars`/`full` (no browser at all) |
| JS-rendered but simple, read-only | obscura (fast to start; render build paints, but a data-driven SPA may stay unmounted — verify the fields before trusting) |
| SPA, Material/React/Stencil, files, screenshots, challenges | **real Chrome** (`--remote-debugging-port=9222`) |

**Do not assume "modern site ⇒ needs a browser".** Most storefronts and
news sites are server-rendered (Next/Laravel/Atcom/Rails): the full HTML already
holds every row, and a browser buys you nothing but 3–10× the latency.
Cheap test, one call:

```
webrain_scrape(url, max_chars=2000)   -> needs_js + truncated
```

- `needs_js:false` **and** the products/text are in `text` ⇒ stay browserless, extract with `base_selector` (this fetches the whole body for you).
- `needs_js:true` ⇒ the visible text is an app shell; go to a browser.

Measured on Greek e-shops (2026-10-24): Sklavenitis 427 KB in **266 ms** and
My Market 830 KB in **415 ms** browserlessly, versus 800–2000 ms per tab in
Chrome. `webrain_scrape(base_selector=...)` returned all 24 tuna rows in one
HTTP request.


### 2. Discover (1 navigate + 2 evals, not 20 navigates)
```
webrain_navigate(url)             -> read `challenge` (must be null) and `links`
webrain_extract(mode=autoschema)  -> candidate repeated containers + counts
webrain_eval(js=one card outerHTML)-> the exact class names / data-attributes
```
Prefer **data-attributes over CSS classes**: frameworks ship
`data-*="<json>"` payloads (name, price, id) that survive restyling. A CSS
class is an API only until the next redesign.

Write the extractor as **one JS expression returning `JSON.stringify(...)`**.
`eval` parses a JSON string into `data`; a plain string lands in `text`.

```js
(() => [...document.querySelectorAll('CARD')].map(c => ({
  n: c.querySelector('TITLE')?.innerText.trim(),
  u: c.querySelector('a')?.href,
  p: JSON.parse(c.dataset.payload || '{}').price
})).filter(x => x.n))()
```

### 3. Batch (one call, N URLs, schema unchanged)
```
webrain_batch(op=eval,
  js=<the extractor>,
  urls=[...],
  concurrency=5,
  wait_selectors=["<sel A>", "<sel B>", ...],  # index-parallel to urls
  output="output/<slug>/data/raw.json")       # ALWAYS pass output
```
**`output:` is the point.** Without it the result lands in a temp file that the
session may GC, and you end up copying paths around. With it, the payload is on
disk for the aggregation step and as an audit trail.

### 4. Persist + aggregate in one script
Parsing 200 JSON rows by eye costs more tokens than the scrape. Write **one**
script (PowerShell/Python/node) that reads `raw.json`, normalises, and emits the
deliverable files. Keep it next to the data so the run is reproducible.

### 5. Verify before reporting
- non-zero rows, expected field coverage (how many rows have the key field?)
- spot-check 1 row against the page
- every claim in the report carries its source URL

## Do / Don't

| Do | Instead of |
|---|---|
| `op=eval` with your own JSON extractor | `op=fetch` + re-reading text |
| `output=` on every batch | copying temp `content.json` paths |
| schema discovered from `autoschema` + one `outerHTML` | guessing selectors from memory |
| `op=eval` for "extract after acting" | `op=interact` returning innerText |
| per-host batches | mixing hosts in one batch |
| 1 discovery navigate | N navigate calls before the batch |

## Hard-won specifics

- **`wait_selectors` is index-parallel to `urls`** — one batch can span hosts
  that render differently. Verified: Sklavenitis (`div.product`) and My Market
  (`article.product--teaser`) in ONE call, 2.0 s and 1.3 s. `wait_timeout_secs`
  is still one value for the whole call, so size it for the slowest host.
- **A Chrome restart invalidates the MCP's CDP socket** (`CDP write failed:
  channel closed`, ms=0). Restart the MCP server after restarting the browser;
  it does not re-dial on its own.
- **`interact` returns the interaction's own return value** (JSON → `data`).
  Return the data you acted for; fall back to innerText only when you return
  nothing. If a site's data only appears after clicks/scroll, put the clicks and
  the read in the *same* JS and use `op=eval`.
- **`cdp_urls` runs backends sequentially** — adding browsers does not speed a
  job up, `concurrency` does.
- **`needs_js: true` from `webrain_scrape`** means the visible text is the app
  shell; switch engines, don't parse it.
- **SPA wait pattern:** `wait_selector` on a card element (e.g. `odsc-tile`,
  `.product`) beats any sleep. A 1.4 s empty result means "not rendered yet",
  not "no data".
- **Zero rows is a finding, not a failure** — but confirm it isn't a block page
  first (check `challenge`, then `observe(what=clean)`).

## Escalate, never grind

Stop and ask the human when:
`challenge` is non-null · the site needs a **store/postcode/account** selection
before it will show data · a CAPTCHA the native path can't claim · a login you
don't have credentials for.

Say exactly what you need ("select a store in the Chrome window that is already
open, then say go") and record the gap in the report. Never invent numbers to
fill it.

## Token discipline

Don't dump HTML or full tool payloads into the conversation. Batches go to
`output/`, summaries come back. Aim for: 1 discovery navigate, ≤3 evals, 1–N
batches, 1 script, 1 report.
