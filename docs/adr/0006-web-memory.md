# ADR 0006 — Web-memory: a bounded domain index, not a completeness promise

- Status: proposed (Phase 2/4 foundation landed on `feat/web-memory`)
- Date: 2026-11-26
- Supersedes: none
- Builds on: PR #116 (`feat/domain-links` crawler foundation)

## Context

`codebase-memory-mcp` indexes a finite repository into a SQLite graph with
tree-sitter ASTs, BM25/FTS5 and embeddings, then serves structural queries to an
LLM. A domain is an open, mutating graph: URLs are unbounded, pages change,
sitemaps are incomplete, and subdomains are not necessarily linked. The same
architecture applies (parse → graph → index → serve), but the product claim must
change from “indexed the whole repo” to “measured coverage under a crawl SLO”.

## Decision

Add an **optional** web-memory store to webrain, reusing the existing crawler:

- SQLite (WAL) graph tables: `hosts`, `urls`, `edges`, `pages`, `sitemaps`,
  `runs`, durable `jobs`.
- FTS5 `pages_fts` BM25 index over page title/body; re-indexing a URL replaces
  its old row (content-hash based in later phases).
- Coverage is reported as discovered / fetched / indexed counts and percentages
  per domain (including `*.domain` subdomains), never as “100%”.
- The feature is optional; the default binary and existing tools stay unchanged.

Phases (all tracked in this goal):

0. SLO + coverage report (discovered/fetched/indexed/freshness per host).
1. Discovery: CT (`crt.sh`), DNS enumeration, per-host robots/sitemap, optional
   Common Crawl/Wayback seeds; `mode:"domain"` with `subdomains/use_ct/use_sitemap`.
2. Persistent crawl store and durable frontier (this ADR’s `webmemory` module).
3. Fetch/render: HTTP-first, JS detection → browser fallback, auth/session/vault,
   CAPTCHA escalation, robots compliance; link-graph edges.
4. Keyword search: FTS5 BM25 with domain/subdomain/path/lang filters, snippets;
   optional text embeddings (external `EMBED_*` or a small bundled model).
5. Operations: daemon/queue, coverage dashboards, golden-corpus tests (docs site
   vs sitemap ground truth), disk/memory budgets.

## Schema (Phase 2/4 foundation)

`hosts(host, first_seen, last_seen)`, `urls(url, host, discovered_at, fetched_at,
status, title, content_hash, depth, indexed)`, `edges(from_url, to_url, rel)`,
`pages(url, host, title, lang, text, fetched_at)`, `pages_fts(url, host, title,
body)`, `sitemaps(host, url, lastmod)`, `runs(id, domain, timestamps, counts)`,
`jobs(id, url, host, state, attempts, next_at, last_error)`.

## Non-goals / limits

- Not a search engine for the entire web; only URLs the crawler discovered.
- No guarantee for private/auth-only, unlinked, JS-unrendered, or rate-limited
  content; credentials and rendering policy are explicit inputs.
- No bundled embedding model initially; BM25 first, embeddings optional.
- No claim that codebase-memory’s binary can index websites — the architecture
  is reused, not the tree-sitter code pipeline.

## Consequences

- Adds `rusqlite` (bundled) to `webrain-core`; SQLite FTS5 is verified at test
  time by the `webmemory` tests.
- New tool surface (`index_domain`, `search_pages`, `site_architecture`,
  `index_status`, `link_trace`) is added in Phase 4/5 behind the same feature.
- Every crawl can publish a coverage report; “full” becomes an SLO with evidence.
