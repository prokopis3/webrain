---
name: domain-links
version: "1.0.0"
description: Find every link on a page or across a whole domain with the webrain MCP tools — internal and external anchors, images (lazy/srcset/picture/icons/og), CSS background images, video/audio/embed URLs, stylesheet/script/iframe/form/preload/meta-refresh documents, and the URLs the page actually requested at the network layer. Covers sitemap-first and no-sitemap crawling, HTTP-only link discovery for static sites, browser fallback for JS, external-link collection, canonicalisation/tracking parameters, caps and robustness (robots, throttling, challenges, max pages/depth). Use for link audits, SEO inventories, sitemap discovery, media/background inventories, broken-link checks, API/endpoint discovery, or "get all links from this site/domain".
argument-hint: "<domain or URL> [internal|external|media|documents|network|all]"
allowed-tools: Bash, Read
---

# /domain-links

Gather links with the smallest number of page loads. The tools are split by
boundary, not by link kind, so the recipe matters more than any single call.

## Tool map

| Need | Call |
|---|---|
| Everything a **single page** exposes | `webrain_assets {max: 2000}` (or `webrain_observe {what:"assets"}`) — one JS pass: `links.internal`, `links.external`, `links.other_schemes`, `images` (incl. `background`), `videos`, `documents`, `network`, and `counts` |
| Only one class | `webrain_assets {only:"links"\|"images"\|"videos"\|"documents"\|"network", max: 2000}` |
| Every URL the page **requested** (XHR/fetch/API/CSS/media), one page | `webrain_network {op:"log", url, wait_ms}` then `op:"requests"`; add `type:"XHR"` / `filter` |
| Every URL the page requested, **many pages** | `webrain_batch {op:"network", urls:[…], concurrency, max}` |
| All link classes, **many pages** | `webrain_batch {op:"assets", urls:[…], concurrency, max}` |
| Every URL the site **publishes** | `webrain_crawl {mode:"sitemap", url:"https://site"}` — pure HTTP, robots → sitemap index → leaves |
| Crawl the **internal link graph** | `webrain_crawl {mode:"spider", seed_url, same_domain:true, max_depth, max_pages, concurrency}` |
| Same, static site, **no browser** | add `http_only:true, no_content:true` — pooled HTTP GET + `scraper`, no tab/JS |
| Seed from the published inventory | add `seed_from_sitemap:true` (works with both spider paths; falls back to the seed URL) |
| Custom one-off | `webrain_eval` (one page) or `webrain_batch {op:"eval"}` (many) |

`webrain_crawl mode=spider` returns one row per page: `{page:{url,title,content,error,…}, depth, links:[…]}`.

## Recipes

### 1. One page — every link class

```json
{"name":"webrain_assets","args":{"max":2000}}
```

Read `counts` first (`internal`, `external`, `images`, `videos`,
`documents`, `network`, `scanned`). Counts are **pre-cap**; the arrays are capped
per `max`. `documents` = stylesheets, scripts, preloads, iframes, forms,
meta-refresh, manifest; `network` = Resource-Timing URLs with initiator type,
transfer size and duration. Background images are in `images` with
`kind:"background"`.

To collect the real network layer instead (API endpoints, lazy-loaded assets),
use `webrain_network {op:"log", url, wait_ms:3000}` **on the same page**, then
`op:"requests"` (it returns status/type/bytes; `op:"request"` fetches one body).

### 2. Whole domain, sitemap exists (fastest, no browser)

```json
{"name":"webrain_crawl","args":{"mode":"sitemap","url":"https://site"}}
```

Then, in batches:

```json
{"name":"webrain_batch","args":{
  "op":"assets","urls":["https://site/a","https://site/b"],
  "concurrency":6,"max":300}}
```

Use `op:"network"` instead when you need request URLs rather than DOM/CSS links.
The sitemap gives the URL inventory; the batch gives every link class on each
page. This is the cheapest path: the sitemap walk is plain HTTP, and the batch
runs one tab per URL only for the pages you actually need.

### 3. Whole domain, no sitemap

```json
{"name":"webrain_crawl","args":{
  "mode":"spider","seed_url":"https://site",
  "same_domain":true,"max_depth":3,"max_pages":500,
  "http_only":true,"no_content":true,"seed_from_sitemap":true,
  "concurrency":8,"respect_robots":true}}
```

`http_only:true` uses the pooled HTTP agent + `scraper` for `<a>/<area>` — no
tab, no JS, no wait; good for static sites, docs, shops, blogs. The frontier
dedupes on path + sorted query and strips tracking parameters (`utm_*`,
`fbclid`, `gclid`, …), and hash routes (`#/x`) are kept. `seed_from_sitemap`
tries robots/sitemap first and falls back to the seed URL.

If pages are JS-rendered (SPA, infinite scroll, framework hydration), run the
same call with `http_only:false` (browser path) and a smaller
`concurrency`; use `no_content:true` to skip text extraction and get links only.

### 4. External links and mixed internal/external

`webrain_crawl` follows only the seeds' domain by default
(`same_domain:true`). To collect **external** links too, use
`webrain_assets` on the pages: `links.external` is returned explicitly, with
`counts.external`. For a domain-scale external audit, run the sitemap + batch
`op:"assets"` recipe and merge `links.external` from every page.

### 5. Media, CSS backgrounds and documents

`webrain_assets` covers: `<img>` (+lazy/srcset/picture), icons/og/twitter,
CSS `background-image`, `<video>/<audio>`/posters/sources, known embeds, media
links, file links, plus the `documents` and `network` groups. `webrain_get_images`
is the images-only shortcut. For HLS/DASH or API media, `webrain_network` sees the
URLs that were actually fetched even when they are not in the DOM.

### 6. Sitemap-only / link-health

- `webrain_crawl {mode:"sitemap", url}` → all published URLs.
- `webrain_crawl {mode:"validate", urls:[…]}` → alive/dead probe, no browser.
- `webrain_crawl {mode:"scan", url, max_scrolls}` → infinite-scroll pages, when
  the links only appear after scrolling.

## Speed rules

1. **Sitemap before spider.** A sitemap walk is HTTP-only and removes most of
   the discovery work. If there is no sitemap, say so and use the spider.
2. **`http_only:true` for static pages.** No tab, no JS, no 300 ms settle.
   Switch to the browser only when the page is a JS app.
3. **One JS pass for one page.** `webrain_assets` returns every class in one
   evaluate; do not call `eval` once per class.
4. **Batch, don't loop.** `webrain_batch op=assets|network` with
   `concurrency` (4–8 typical) is for N pages; a per-URL loop re-attaches the
   browser for every page.
5. **Cap deliberately.** `max` truncates arrays; `counts` stays true. For a
   first pass use `max:200`; for a full page inventory use `max:2000`.
6. **Respect the site.** `respect_robots:true`; `delay_ms` /
   `autothrottle` on rate-limited origins; `max_pages`/`max_depth` bound the
   crawl; `crawl_timeout_secs` for a hard stop.

## Robustness rules

- Check `error` per page; a challenge/consent/login page is not a link source.
  Use real Chrome + a persistent profile/session for protected pages, not the
  HTTP path.
- A sitemap may be missing, gzipped, an index, or stale: the sitemap walk
  follows `<sitemapindex>` recursively and reports `sources`; if it returns no
  URLs, fall back to the spider.
- `http_only` cannot see client-rendered links. If a page returns almost no
  links or a sparse shell, retry it with the browser path.
- `network`/Resource Timing only sees what the page actually requested;
  DOM/CSS links that were never fetched appear in `assets` but not `network`.
  Conversely, `assets` cannot see API endpoints that only exist in a fetch
  response — use `network`/`webrain_network` for those.
- Sitemap/spider URLs are candidates, not proof of content: validate before
  counting a link as live (`mode:"validate"` or a small batch fetch).
- Keep the two outputs separate when reporting: **internal**, **external**,
  **media**, **documents**, **network**, and **other schemes**; a single
  "all links" number hides which class is missing.
