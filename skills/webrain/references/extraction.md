# Extraction

## Strategy ladder (cheapest first)
autoschema → structured schema extraction → targeted eval → JSON-LD → tables →
regex → raw HTML (LAST resort — never for page text).

## Decision matrix
| Task | Tool |
|---|---|
| structured list + schema known | `webrain_extract(mode=schema, base_selector, fields)` |
| schema unknown | `webrain_extract(mode=autoschema)` → container; `eval` → probe fields; then mode=schema |
| paginated 1..N / many URLs | `webrain_batch(op=extract, urls, base_selector, fields, concurrency=8)` |
| URLs unknown (discover) | `eval` → pagination hrefs (same-prefix numeric + next/prev, NO hardcoded classes) → range → batch |
| whole site | `webrain_crawl(mode=spider)` |
| emails / phones / prices / patterns | `webrain_extract(mode=regex)` |
| JSON-LD / microdata | `webrain_extract(mode=jsonld)` |
| tables | `webrain_extract(mode=table)` |
| infinite scroll / load-more | `webrain_crawl(mode=scan)` then extract; or the `/ajax/` offset shortcut below |
| search | `webrain_search` |
| relevance filter | `webrain_extract(mode=bm25)` |

## From-scratch discovery (schema + URLs unknown)
1. `webrain_navigate(seed)` — read `links` + `challenge`.
2. Derive urls from `links` / pagination (eval) → range.
3. `extract(mode=autoschema)` → container selector.
4. `eval` → descendant tags/classes + samples → fields.
5. `webrain_batch(op=extract, urls, base_selector, fields, concurrency=8)` → read
   the parsed `data` array (don't parse `text`).
6. done("Extracted N items across M pages").

## Load-more / infinite-scroll shortcut (fastest path)
These pages usually back the button/observer with a plain JSON/HTML endpoint
(scrapingcourse uses `/ajax/products?offset=N`). Find it via `eval` (grep
`/ajax/` in script tags), then `webrain_batch(op=extract, urls=[...offset
windows...], base_selector, fields)` directly — one call, no interaction, no
scroll. Dedupe overlapping offset windows.

## Token discipline
- Never return raw HTML. `observe(what=state|fit|clean)` + the extractors give
  text/structure far cheaper. `observe(what=html)` is LAST RESORT.
- `webrain_batch(op=extract)` returns each result's products as a parsed `data`
  array — read `data`, don't parse `text`.
- Batch before loop. Filter before summarizing (`bm25`).

## Extraction verification
- 0 items → check `challenge` (block page?), re-run autoschema, or wait for
  JS/scroll then retry.
- Never extract a challenge/login/consent page as target content.
