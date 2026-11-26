//! Web-memory: a persistent SQLite graph + FTS5 index for domains the crawler
//! has seen. This is the storage foundation for `webrain_index_domain` /
//! `webrain_search_domain`: hosts/URLs/edges are the site graph, pages are the
//! content store, and `pages_fts` (FTS5) is the BM25 keyword index.
//!
//! It is intentionally a *web* graph, not a code graph: nodes are hosts, URLs,
//! pages and assets; edges are links/redirects/embeds/forms/sitemap relations.
//! Coverage is measured, never promised — `coverage()` reports discovered,
//! fetched and indexed counts so a caller can see what the crawl did not reach.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::engines::{browser_req, http_agent};

const SCHEMA: &str = r#"
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS hosts (
    host        TEXT PRIMARY KEY,
    first_seen  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS urls (
    url          TEXT PRIMARY KEY,
    host         TEXT NOT NULL,
    discovered_at INTEGER NOT NULL,
    fetched_at   INTEGER,
    status       INTEGER,
    title        TEXT,
    content_hash TEXT,
    depth        INTEGER NOT NULL DEFAULT 0,
    indexed      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS urls_host ON urls(host);
CREATE INDEX IF NOT EXISTS urls_fetched ON urls(fetched_at);

CREATE TABLE IF NOT EXISTS edges (
    from_url TEXT NOT NULL,
    to_url   TEXT NOT NULL,
    rel      TEXT NOT NULL,
    PRIMARY KEY (from_url, to_url, rel)
);
CREATE INDEX IF NOT EXISTS edges_from ON edges(from_url);
CREATE INDEX IF NOT EXISTS edges_to ON edges(to_url);

CREATE TABLE IF NOT EXISTS pages (
    url        TEXT PRIMARY KEY,
    host       TEXT NOT NULL,
    title      TEXT NOT NULL DEFAULT '',
    lang       TEXT NOT NULL DEFAULT '',
    text       TEXT NOT NULL DEFAULT '',
    fetched_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS pages_host ON pages(host);

CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
    url UNINDEXED,
    host UNINDEXED,
    title,
    body,
    tokenize='unicode61'
);

CREATE TABLE IF NOT EXISTS sitemaps (
    host   TEXT NOT NULL,
    url    TEXT NOT NULL,
    lastmod TEXT,
    PRIMARY KEY (host, url)
);

CREATE TABLE IF NOT EXISTS runs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    domain      TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    discovered  INTEGER NOT NULL DEFAULT 0,
    fetched     INTEGER NOT NULL DEFAULT 0,
    indexed     INTEGER NOT NULL DEFAULT 0,
    status      TEXT NOT NULL DEFAULT 'running'
);

CREATE TABLE IF NOT EXISTS jobs (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    url        TEXT NOT NULL,
    host       TEXT NOT NULL,
    state      TEXT NOT NULL DEFAULT 'queued',
    attempts   INTEGER NOT NULL DEFAULT 0,
    next_at    INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);
CREATE INDEX IF NOT EXISTS jobs_ready ON jobs(state, next_at);
"#;

#[derive(Debug, Clone, Serialize)]
pub struct WebPage {
    pub url: String,
    pub host: String,
    pub title: String,
    pub lang: String,
    pub text: String,
    pub content_hash: String,
    pub status: u16,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub url: String,
    pub host: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

pub struct WebMemory {
    conn: Connection,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Spider content is already capped (~3000 chars); keep the store consistent.
const PAGE_TEXT_CAP: usize = 3000;

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// FTS5 is not a full query language here: turn user keywords into safe
/// prefix terms, so `rust crawler` means both words and punctuation cannot
/// produce a syntax error.
fn fts_query(q: &str) -> String {
    let terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.replace('"', "")))
        .collect();
    terms.join(" AND ")
}

impl WebMemory {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open web-memory {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA).context("web-memory schema")?;
        Ok(Self { conn })
    }

    pub fn upsert_host(&self, host: &str) -> Result<()> {
        let t = now();
        self.conn.execute(
            "INSERT INTO hosts(host, first_seen, last_seen) VALUES(?1, ?2, ?2)
             ON CONFLICT(host) DO UPDATE SET last_seen=excluded.last_seen",
            params![host, t],
        )?;
        Ok(())
    }

    /// Record a URL as discovered; repeated discovery keeps the first seen time.
    pub fn discover_url(&self, url: &str, host: &str, depth: usize) -> Result<()> {
        let t = now();
        self.upsert_host(host)?;
        self.conn.execute(
            "INSERT INTO urls(url, host, discovered_at, depth) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(url) DO NOTHING",
            params![url, host, t, depth as i64],
        )?;
        Ok(())
    }

    pub fn mark_fetched(&self, url: &str, status: u16, title: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE urls SET fetched_at=?2, status=?3, title=COALESCE(?4, title) WHERE url=?1",
            params![url, now(), status as i64, title],
        )?;
        Ok(())
    }

    pub fn add_edge(&self, from_url: &str, to_url: &str, rel: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO edges(from_url, to_url, rel) VALUES(?1, ?2, ?3)
             ON CONFLICT(from_url, to_url, rel) DO NOTHING",
            params![from_url, to_url, rel],
        )?;
        Ok(())
    }

    pub fn record_sitemap(&self, host: &str, url: &str, lastmod: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sitemaps(host, url, lastmod) VALUES(?1, ?2, ?3)
             ON CONFLICT(host, url) DO UPDATE SET lastmod=excluded.lastmod",
            params![host, url, lastmod],
        )?;
        Ok(())
    }

    /// Insert/replace a page and its FTS row. Re-indexing a changed page replaces
    /// the previous text instead of duplicating it.
    pub fn upsert_page(&self, page: &WebPage) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO pages(url, host, title, lang, text, fetched_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(url) DO UPDATE SET
               host=excluded.host, title=excluded.title, lang=excluded.lang,
               text=excluded.text, fetched_at=excluded.fetched_at",
            params![
                page.url,
                page.host,
                page.title,
                page.lang,
                page.text,
                page.fetched_at
            ],
        )?;
        tx.execute("DELETE FROM pages_fts WHERE url=?1", params![page.url])?;
        tx.execute(
            "INSERT INTO pages_fts(url, host, title, body) VALUES(?1, ?2, ?3, ?4)",
            params![page.url, page.host, page.title, page.text],
        )?;
        tx.execute(
            "UPDATE urls SET fetched_at=?2, status=?3, title=?4, content_hash=?5, indexed=1
             WHERE url=?1",
            params![
                page.url,
                page.fetched_at,
                page.status as i64,
                page.title,
                page.content_hash
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// BM25 keyword search, optionally restricted to one host or its
    /// subdomains (`blog.example.com` matches domain `example.com`).
    pub fn search(
        &self,
        query: &str,
        domain: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let fts = fts_query(query);
        if fts.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let domain = domain.unwrap_or("").trim().to_lowercase();
        let (sql, args): (String, Vec<Box<dyn rusqlite::ToSql>>) = if domain.is_empty() {
            (
                "SELECT url, host, title, snippet(pages_fts, 3, '[', ']', '…', 12),
                        bm25(pages_fts) AS score
                 FROM pages_fts WHERE pages_fts MATCH ?1
                 ORDER BY score LIMIT ?2"
                    .to_string(),
                vec![Box::new(fts), Box::new(limit as i64)],
            )
        } else {
            (
                "SELECT url, host, title, snippet(pages_fts, 3, '[', ']', '…', 12),
                        bm25(pages_fts) AS score
                 FROM pages_fts
                 WHERE pages_fts MATCH ?1
                   AND (host = ?2 OR host LIKE ?3)
                 ORDER BY score LIMIT ?4"
                    .to_string(),
                vec![
                    Box::new(fts),
                    Box::new(domain.clone()),
                    Box::new(format!("%.{domain}")),
                    Box::new(limit as i64),
                ],
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), |r| {
            Ok(SearchHit {
                url: r.get(0)?,
                host: r.get(1)?,
                title: r.get(2)?,
                snippet: r.get(3)?,
                score: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Coverage for a domain (host + subdomains): discovered/fetched/indexed
    /// URLs. This is the SLO report, not a completeness claim.
    pub fn coverage(&self, domain: &str) -> Result<serde_json::Value> {
        let pattern = format!("%.{}", domain.trim().to_lowercase());
        let (discovered, fetched, indexed): (i64, i64, i64) = self.conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN fetched_at IS NOT NULL THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(indexed), 0)
             FROM urls WHERE host=?1 OR host LIKE ?2",
            params![domain.trim().to_lowercase(), pattern],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(serde_json::json!({
            "domain": domain,
            "discovered": discovered,
            "fetched": fetched,
            "indexed": indexed,
            "fetched_pct": pct(fetched, discovered),
            "indexed_pct": pct(indexed, discovered),
        }))
    }

    /// Host/page/link counts, plus the top hosts and most-linked targets.
    pub fn architecture(&self, domain: &str) -> Result<serde_json::Value> {
        let pattern = format!("%.{}", domain.trim().to_lowercase());
        let hosts: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM hosts WHERE host=?1 OR host LIKE ?2",
            params![domain.trim().to_lowercase(), pattern],
            |r| r.get(0),
        )?;
        let urls: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM urls WHERE host=?1 OR host LIKE ?2",
            params![domain.trim().to_lowercase(), pattern],
            |r| r.get(0),
        )?;
        let pages: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pages WHERE host=?1 OR host LIKE ?2",
            params![domain.trim().to_lowercase(), pattern],
            |r| r.get(0),
        )?;
        let edges: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM edges e JOIN urls u ON u.url=e.from_url
             WHERE u.host=?1 OR u.host LIKE ?2",
            params![domain.trim().to_lowercase(), pattern],
            |r| r.get(0),
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT host, COUNT(*) AS n FROM urls
             WHERE host=?1 OR host LIKE ?2 GROUP BY host ORDER BY n DESC LIMIT 20",
        )?;
        let top_hosts: Vec<serde_json::Value> = stmt
            .query_map(params![domain.trim().to_lowercase(), pattern], |r| {
                Ok(serde_json::json!({"host": r.get::<_, String>(0)?, "urls": r.get::<_, i64>(1)?}))
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(serde_json::json!({
            "domain": domain,
            "hosts": hosts,
            "urls": urls,
            "pages": pages,
            "edges": edges,
            "top_hosts": top_hosts,
        }))
    }

    /// Start a crawl run; returns its id.
    /// Feed a completed spider crawl into the store: discover every URL, index
    /// page text (spider content is capped, see `PAGE_TEXT_CAP`), and record
    /// `LINKS_TO` edges. Returns the resulting coverage report.
    pub fn ingest_crawl(
        &self,
        domain: &str,
        results: &[crate::engines::SpiderResult],
    ) -> Result<serde_json::Value> {
        use sha2::{Digest, Sha256};
        let mut indexed = 0usize;
        for result in results {
            let page = &result.page;
            let url = page.url.as_str();
            let Some(host) = host_of(url) else { continue };
            self.discover_url(url, &host, result.depth)?;
            if let Some(content) = page.content.as_deref() {
                let text: String = content.chars().take(PAGE_TEXT_CAP).collect();
                let hash = format!("{:?}", Sha256::digest(text.as_bytes()));
                self.upsert_page(&WebPage {
                    url: url.to_string(),
                    host: host.clone(),
                    title: page.title.clone().unwrap_or_default(),
                    lang: String::new(),
                    text,
                    content_hash: hash,
                    status: 200,
                    fetched_at: now(),
                })?;
                indexed += 1;
            } else {
                self.mark_fetched(url, 0, page.title.as_deref())?;
            }
            for link in &result.links {
                let Some(target_host) = host_of(link) else {
                    continue;
                };
                self.discover_url(link, &target_host, result.depth + 1)?;
                self.add_edge(url, link, "LINKS_TO")?;
            }
        }
        let mut report = self.coverage(domain)?;
        report["indexed_this_run"] = serde_json::json!(indexed);
        Ok(report)
    }

    pub fn start_run(&self, domain: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO runs(domain, started_at) VALUES(?1, ?2)",
            params![domain, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_run(&self, id: i64, discovered: i64, fetched: i64, indexed: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET finished_at=?2, discovered=?3, fetched=?4, indexed=?5,
                    status='finished' WHERE id=?1",
            params![id, now(), discovered, fetched, indexed],
        )?;
        Ok(())
    }

    /// Queue a URL for the durable frontier (reserved for Phase 2 worker mode).
    pub fn enqueue(&self, url: &str, host: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO jobs(url, host, state, next_at) VALUES(?1, ?2, 'queued', ?3)",
            params![url, host, now()],
        )?;
        Ok(())
    }

    /// Claim up to `n` ready jobs, marking them `running`.
    pub fn claim_jobs(&self, n: usize) -> Result<Vec<(i64, String, String)>> {
        let t = now();
        let mut stmt = self.conn.prepare(
            "SELECT id, url, host FROM jobs WHERE state='queued' AND next_at<=?1
             ORDER BY id LIMIT ?2",
        )?;
        let ids: Vec<(i64, String, String)> = stmt
            .query_map(params![t, n as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        for (id, _, _) in &ids {
            self.conn.execute(
                "UPDATE jobs SET state='running', attempts=attempts+1 WHERE id=?1",
                params![id],
            )?;
        }
        Ok(ids)
    }
}

fn pct(n: i64, total: i64) -> f64 {
    if total <= 0 {
        0.0
    } else {
        ((n as f64 / total as f64) * 1000.0).round() / 10.0
    }
}

/// Normalize/validate the registrable domain used for discovery. Kept
/// deliberately strict: no scheme, path, wildcard or userinfo reaches crt.sh.
pub fn normalize_domain(domain: &str) -> Result<String> {
    let d = domain.trim().trim_matches('.').to_ascii_lowercase();
    if d.is_empty() || !d.contains('.') {
        anyhow::bail!("domain must contain a dot: {domain:?}");
    }
    if !d
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        || d.split('.').any(|label| label.is_empty())
    {
        anyhow::bail!("invalid domain: {domain:?}");
    }
    Ok(d)
}

/// Certificate Transparency discovery through crt.sh. Returns subdomains only
/// (wildcards and the apex are excluded). Network failures are returned as an
/// error; callers should still keep DNS results.
pub fn ct_subdomains(domain: &str) -> Result<Vec<String>> {
    let d = normalize_domain(domain)?;
    let url = format!("https://crt.sh/?q=%25.{d}&output=json");
    let resp = browser_req(http_agent(None)?.get(&url)).call()?;
    let status = resp.status().as_u16();
    let body = resp.into_body().read_to_string()?;
    if !(200..300).contains(&status) {
        anyhow::bail!("crt.sh GET -> {status}");
    }
    Ok(parse_ct_names(&body, &d))
}

/// Parse crt.sh's JSON without trusting ordering or duplicate entries. Each row
/// carries newline-separated `name_value` (and often `common_name`).
pub fn parse_ct_names(body: &str, domain: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(rows) = root.as_array() else {
        return Vec::new();
    };
    let domain = domain.trim().trim_matches('.').to_ascii_lowercase();
    let suffix = format!(".{domain}");
    let mut names = std::collections::BTreeSet::new();
    for row in rows {
        for key in ["name_value", "common_name"] {
            let Some(raw) = row.get(key).and_then(|v| v.as_str()) else {
                continue;
            };
            for line in raw.lines() {
                let name = line.trim().trim_end_matches('.').to_ascii_lowercase();
                if name.is_empty() || name.starts_with("*.") {
                    continue;
                }
                if name.ends_with(&suffix) && name != domain {
                    names.insert(name);
                }
            }
        }
    }
    names.into_iter().collect()
}

/// DNS brute-force probe over a small caller-supplied label list. This is not a
/// resolver or zone-transfer tool: it only reports labels that resolve. Kept
/// synchronous and bounded; callers choose the wordlist.
pub fn dns_subdomains(domain: &str, labels: &[&str]) -> Vec<String> {
    use std::net::ToSocketAddrs;
    let Ok(domain) = normalize_domain(domain) else {
        return Vec::new();
    };
    let mut out = std::collections::BTreeSet::new();
    for label in labels {
        let label = label.trim().trim_matches('.');
        if label.is_empty() || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            continue;
        }
        let host = format!("{label}.{domain}");
        if (host.as_str(), 80u16).to_socket_addrs().is_ok() {
            out.insert(host);
        }
    }
    out.into_iter().collect()
}

/// Phase 1 discovery entry point: CT + optional DNS labels, deduped and
/// reported with per-source errors. An empty result is evidence, not a promise
/// that no subdomain exists.
pub fn discover_subdomains(domain: &str, use_ct: bool, dns_labels: &[&str]) -> serde_json::Value {
    let mut names = std::collections::BTreeSet::new();
    let mut errors = Vec::new();
    let mut ct_ok = false;
    if use_ct {
        match ct_subdomains(domain) {
            Ok(found) => {
                ct_ok = true;
                names.extend(found);
            }
            Err(e) => errors.push(format!("ct: {e}")),
        }
    }
    let dns = dns_subdomains(domain, dns_labels);
    names.extend(dns.iter().cloned());
    serde_json::json!({
        "domain": domain,
        "subdomains": names,
        "ct_ok": ct_ok,
        "dns_probed": dns_labels.len(),
        "errors": errors,
    })
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn ct_parser_keeps_only_real_subdomains() {
        let body = r#"[
            {"name_value":"api.example.com\n*.example.com\nmail.example.com"},
            {"common_name":"blog.example.com"},
            {"name_value":"evil-example.com\nexample.com"}
        ]"#;
        assert_eq!(
            parse_ct_names(body, "example.com"),
            vec![
                "api.example.com".to_string(),
                "blog.example.com".to_string(),
                "mail.example.com".to_string()
            ]
        );
        assert!(parse_ct_names("not json", "example.com").is_empty());
    }

    #[test]
    fn domain_normalization_rejects_urls_and_wildcards() {
        assert_eq!(normalize_domain(" Example.COM. ").unwrap(), "example.com");
        assert!(normalize_domain("https://example.com").is_err());
        assert!(normalize_domain("*.example.com").is_err());
        assert!(normalize_domain("localhost").is_err());
    }

    #[test]
    fn discovery_without_network_sources_is_empty_but_reports() {
        let v = discover_subdomains("example.com", false, &[]);
        assert_eq!(v["domain"], "example.com");
        assert_eq!(v["subdomains"].as_array().unwrap().len(), 0);
        assert_eq!(v["ct_ok"], false);
        assert_eq!(v["dns_probed"], 0);
        assert_eq!(v["errors"].as_array().unwrap().len(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(url: &str, host: &str, title: &str, text: &str) -> WebPage {
        WebPage {
            url: url.into(),
            host: host.into(),
            title: title.into(),
            lang: "en".into(),
            text: text.into(),
            content_hash: format!("h-{url}"),
            status: 200,
            fetched_at: 1,
        }
    }

    #[test]
    fn fts_search_finds_pages_and_filters_subdomains() {
        let mem = WebMemory::open_in_memory().unwrap();
        mem.upsert_page(&page(
            "https://example.com/a",
            "example.com",
            "Rust crawler",
            "a fast crawler for rust domains",
        ))
        .unwrap();
        mem.upsert_page(&page(
            "https://blog.example.com/b",
            "blog.example.com",
            "Garden notes",
            "tomatoes and basil",
        ))
        .unwrap();

        let hits = mem.search("rust crawler", None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://example.com/a");
        assert!(!hits[0].snippet.is_empty());

        let hits = mem.search("garden", Some("example.com"), 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].host, "blog.example.com");

        assert!(mem.search("!!", None, 10).unwrap().is_empty());
    }

    #[test]
    fn reindex_replaces_old_text() {
        let mem = WebMemory::open_in_memory().unwrap();
        mem.upsert_page(&page("https://e.test/x", "e.test", "Old", "alpha"))
            .unwrap();
        mem.upsert_page(&page("https://e.test/x", "e.test", "New", "beta"))
            .unwrap();
        assert!(mem.search("alpha", None, 10).unwrap().is_empty());
        assert_eq!(mem.search("beta", None, 10).unwrap().len(), 1);
    }

    #[test]
    fn graph_and_coverage_counts() {
        let mem = WebMemory::open_in_memory().unwrap();
        mem.discover_url("https://example.com/", "example.com", 0)
            .unwrap();
        mem.discover_url("https://example.com/a", "example.com", 1)
            .unwrap();
        mem.discover_url("https://blog.example.com/x", "blog.example.com", 1)
            .unwrap();
        mem.add_edge("https://example.com/", "https://example.com/a", "LINKS_TO")
            .unwrap();
        mem.mark_fetched("https://example.com/", 200, Some("Home"))
            .unwrap();
        mem.upsert_page(&page("https://example.com/a", "example.com", "A", "body"))
            .unwrap();

        let cov = mem.coverage("example.com").unwrap();
        assert_eq!(cov["discovered"], 3);
        assert_eq!(cov["fetched"], 2);
        assert_eq!(cov["indexed"], 1);

        let arch = mem.architecture("example.com").unwrap();
        assert_eq!(arch["hosts"], 2);
        assert_eq!(arch["urls"], 3);
        assert_eq!(arch["edges"], 1);
    }

    #[test]
    fn ingests_crawl_results_into_pages_and_edges() {
        let mem = WebMemory::open_in_memory().unwrap();
        let results = vec![crate::engines::SpiderResult {
            page: crate::browser::PageResult {
                url: "https://example.com/".into(),
                title: Some("Home".into()),
                content: Some("a rust crawler for web memory".into()),
                error: None,
                duration_ms: 5,
            },
            depth: 0,
            links: vec!["https://example.com/a".into()],
        }];
        let report = mem.ingest_crawl("example.com", &results).unwrap();
        assert_eq!(report["discovered"], 2);
        assert_eq!(report["indexed"], 1);
        assert_eq!(report["indexed_this_run"], 1);
        assert_eq!(mem.search("rust", None, 10).unwrap().len(), 1);
        assert_eq!(mem.architecture("example.com").unwrap()["edges"], 1);
    }

    #[test]
    fn durable_jobs_are_claimable_once() {
        let mem = WebMemory::open_in_memory().unwrap();
        mem.enqueue("https://e.test/1", "e.test").unwrap();
        mem.enqueue("https://e.test/2", "e.test").unwrap();
        let first = mem.claim_jobs(1).unwrap();
        assert_eq!(first.len(), 1);
        let second = mem.claim_jobs(2).unwrap();
        assert_eq!(second.len(), 1);
        assert_ne!(first[0].1, second[0].1);
    }
}
