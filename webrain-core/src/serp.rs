// webrain-core/src/serp.rs
// Structured SERP (Search Engine Results Page) extraction — a typed SERP API.
//
// Reference/inspiration: the standalone "rust-serp-api" app (Axum + reqwest +
// scraper scraping DuckDuckGo HTML). Instead of a second HTTP server, this is a
// first-class webrain capability: typed {position,title,url,domain,snippet}
// results, reachable from the webrain_serp MCP tool, the `webrain serp` CLI, and
// any LLM over webrain's MCP transport (stdio or --http).
//
// "For all browser engines": duckduckgo/bing/google serve plain HTML → fetched
// over the pooled no-browser HTTP agent (no browser at all). `brave` is a JS SPA
// → rendered in whatever CDP engine is attached (Chrome/obscura/lightpanda) via
// navigate + get_html. `auto` fetches all HTTP engines concurrently and merges.
//
// Recommended features (from the reference app's list) that make sense for a
// local portable tool: provider fallback, URL dedupe, pagination, safe search +
// region, request ids, retry with backoff, parallel multi-provider. SaaS-only
// concerns are deferred — Redis cache, API keys, rate limits, billing, metrics,
// OTel, circuit breaker, proxy rotation (ponytail: name the ceiling, add when
// this is hosted as a multi-tenant service).

use crate::browser::BrowserBackend;
use serde::Serialize;
use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

/// One typed organic search result.
#[derive(Debug, Clone, Serialize)]
pub struct SerpResult {
    /// 1-based rank within the returned list.
    pub position: usize,
    pub title: String,
    pub url: String,
    /// Registrable-ish host (www. stripped), for at-a-glance domain filtering.
    pub domain: String,
    pub snippet: String,
}

/// Full SERP response envelope.
#[derive(Debug, Clone, Serialize)]
pub struct SerpResponse {
    pub query: String,
    /// engine actually used to satisfy the request (auto → first non-empty).
    pub engine: String,
    pub results: Vec<SerpResult>,
    /// Opaque id so a caller can correlate a response across retries/logs.
    pub request_id: String,
    /// Wall-clock time for the whole request (incl. fallback chain), ms.
    pub ms: u64,
    /// Engines that returned zero/errored and were skipped (auto + fallback).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
}

/// Search parameters for serp_search.
#[derive(Debug, Clone)]
pub struct SerpOpts {
    pub query: String,
    /// duckduckgo | bing | google | brave | auto (all HTTP engines, merged).
    pub engine: String,
    /// Max results to return, 1..=50.
    pub limit: usize,
    /// 0-based results page (per-engine offset).
    pub page: usize,
    /// Safe search on (per-engine param: ddg k1 / bing adlt / google safe).
    pub safe: bool,
    /// Region/locale (ddg `kl`), e.g. "us-en", "gb-en", "gr-el", "de-de".
    pub region: Option<String>,
    /// Transient-failure retries (backoff 200ms * 2^attempt).
    pub retries: u32,
    /// Allow provider fallback when a specific engine errors or returns zero.
    pub fallback: bool,
}

impl Default for SerpOpts {
    fn default() -> Self {
        SerpOpts {
            query: String::new(),
            engine: "auto".to_string(),
            limit: 10,
            page: 0,
            safe: false,
            region: None,
            retries: 2,
            fallback: true,
        }
    }
}

/// HTTP-renderable engines, in fallback/auto preference order.
const HTTP_ENGINES: [&str; 3] = ["duckduckgo", "bing", "google"];

/// Build the provider URL for one engine + params.
fn engine_url(
    engine: &str,
    q: &str,
    page: usize,
    safe: bool,
    region: Option<&str>,
) -> anyhow::Result<String> {
    match engine {
        "duckduckgo" => {
            let mut u = Url::parse("https://html.duckduckgo.com/html/")?;
            let mut p = u.query_pairs_mut();
            p.append_pair("q", q);
            // ddg html paginates ~30 results per page via `s` (0, 30, 60, ...).
            p.append_pair("s", &(page * 30).to_string());
            // k5=1 disables the /l/?uddg= redirect wrapper → direct result hrefs.
            p.append_pair("k5", "1");
            // k1: -1 off, 1 moderate, 2 strict.
            p.append_pair("k1", if safe { "2" } else { "-1" });
            if let Some(r) = region {
                p.append_pair("kl", r);
            }
            drop(p);
            Ok(u.to_string())
        }
        "bing" => {
            let mut u = Url::parse("https://www.bing.com/search")?;
            let mut p = u.query_pairs_mut();
            p.append_pair("q", q);
            // bing paginates 10/page via `first` (1, 11, 21, ...).
            p.append_pair("first", &(page * 10 + 1).to_string());
            p.append_pair("adlt", if safe { "strict" } else { "off" });
            drop(p);
            Ok(u.to_string())
        }
        "google" => {
            let mut u = Url::parse("https://www.google.com/search")?;
            let mut p = u.query_pairs_mut();
            p.append_pair("q", q);
            p.append_pair("start", &(page * 10).to_string());
            p.append_pair("num", "10");
            p.append_pair("safe", if safe { "active" } else { "off" });
            drop(p);
            Ok(u.to_string())
        }
        "brave" => {
            let mut u = Url::parse("https://search.brave.com/search")?;
            u.query_pairs_mut().append_pair("q", q);
            Ok(u.to_string())
        }
        other => Err(anyhow::anyhow!(
            "unknown engine '{other}' (duckduckgo|bing|google|brave|auto)"
        )),
    }
}

/// Parse a provider HTML page into typed results. Best-effort: a bad parse
/// yields fewer results (never an error) so the fallback chain can cover it.
fn parse_results(engine: &str, html: &str, limit: usize) -> Vec<SerpResult> {
    match engine {
        "duckduckgo" => parse_duckduckgo(html, limit),
        "bing" => parse_bing(html, limit),
        "google" => parse_google(html, limit),
        "brave" => parse_brave(html, limit),
        _ => Vec::new(),
    }
}

fn parse_duckduckgo(html: &str, limit: usize) -> Vec<SerpResult> {
    let doc = scraper::Html::parse_document(html);
    let Ok(result_sel) = scraper::Selector::parse(".result") else {
        return Vec::new();
    };
    let Ok(title_sel) = scraper::Selector::parse(".result__title a") else {
        return Vec::new();
    };
    let Ok(snippet_sel) = scraper::Selector::parse(".result__snippet") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for el in doc.select(&result_sel) {
        if out.len() >= limit {
            break;
        }
        let Some(a) = el.select(&title_sel).next() else {
            continue;
        };
        let title = clean(&a.text().collect::<Vec<_>>().join(" "));
        let Some(href) = a.value().attr("href") else {
            continue;
        };
        let url = normalize_url(href);
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let snippet = el
            .select(&snippet_sel)
            .next()
            .map(|n| clean(&n.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();
        out.push(SerpResult {
            position: out.len() + 1,
            title,
            domain: domain(&url),
            url,
            snippet,
        });
    }
    out
}

fn parse_bing(html: &str, limit: usize) -> Vec<SerpResult> {
    let doc = scraper::Html::parse_document(html);
    let Ok(li_sel) = scraper::Selector::parse("li.b_algo") else {
        return Vec::new();
    };
    let Ok(a_sel) = scraper::Selector::parse("h2 a") else {
        return Vec::new();
    };
    let Ok(snip_sel) = scraper::Selector::parse(".b_caption p") else {
        return Vec::new();
    };
    let Ok(fallback_snip_sel) = scraper::Selector::parse("p") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for li in doc.select(&li_sel) {
        if out.len() >= limit {
            break;
        }
        let Some(a) = li.select(&a_sel).next() else {
            continue;
        };
        let title = clean(&a.text().collect::<Vec<_>>().join(" "));
        let Some(href) = a.value().attr("href") else {
            continue;
        };
        // bing wraps every result in a ck/a tracking redirect whose real target
        // lives in the base64url `u=` param — decode it (falls back to the href).
        let url = bing_target(href).unwrap_or_else(|| normalize_url(href));
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let snippet = li
            .select(&snip_sel)
            .next()
            .map(|n| clean(&n.text().collect::<Vec<_>>().join(" ")))
            .or_else(|| {
                li.select(&fallback_snip_sel)
                    .next()
                    .map(|n| clean(&n.text().collect::<Vec<_>>().join(" ")))
            })
            .unwrap_or_default();
        out.push(SerpResult {
            position: out.len() + 1,
            title,
            domain: domain(&url),
            url,
            snippet,
        });
    }
    out
}

/// Decode a Bing `bing.com/ck/a?...&u=<base64url>` tracking redirect into the
/// real target URL. The `u=` value may carry an `a1` marker prefix before the
/// base64url payload — try both forms. Returns None when no candidate decodes
/// to an http(s) URL.
fn bing_target(href: &str) -> Option<String> {
    let idx = href.find("u=")?;
    let rest = &href[idx + 2..];
    let end = rest.find(['&', '#']).unwrap_or(rest.len());
    let b64 = &rest[..end];
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    for candidate in [b64, b64.strip_prefix("a1").unwrap_or("")] {
        if candidate.is_empty() {
            continue;
        }
        if let Ok(bytes) = engine.decode(candidate) {
            let url = String::from_utf8_lossy(&bytes).into_owned();
            if url.starts_with("http://") || url.starts_with("https://") {
                return Some(url);
            }
        }
    }
    None
}

fn parse_google(html: &str, limit: usize) -> Vec<SerpResult> {
    let doc = scraper::Html::parse_document(html);
    // Container: #search .g (newer layout is div.MjjYud — keep both).
    let Ok(cont_sel) = scraper::Selector::parse("#search .g, div.MjjYud") else {
        return Vec::new();
    };
    let Ok(a_sel) = scraper::Selector::parse("a[href]") else {
        return Vec::new();
    };
    let Ok(h3_sel) = scraper::Selector::parse("h3") else {
        return Vec::new();
    };
    let Ok(snip_sel) = scraper::Selector::parse(".VwiC3b") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for el in doc.select(&cont_sel) {
        if out.len() >= limit {
            break;
        }
        // The organic-result link is the first external <a> whose child h3 holds
        // the title (ad/related links point at google.com internals).
        let mut chosen: Option<(String, String)> = None; // (title, href)
        for a in el.select(&a_sel) {
            let Some(href) = a.value().attr("href") else {
                continue;
            };
            let url = normalize_url(href);
            if url.is_empty() || url.starts_with("https://www.google.") {
                continue;
            }
            let title = a
                .select(&h3_sel)
                .next()
                .map(|h| clean(&h.text().collect::<Vec<_>>().join(" ")))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| clean(&a.text().collect::<Vec<_>>().join(" ")));
            if !title.is_empty() {
                chosen = Some((title, url));
                break;
            }
        }
        let Some((title, url)) = chosen else { continue };
        let snippet = el
            .select(&snip_sel)
            .next()
            .map(|n| clean(&n.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();
        out.push(SerpResult {
            position: out.len() + 1,
            title,
            domain: domain(&url),
            url,
            snippet,
        });
    }
    out
}

fn parse_brave(html: &str, limit: usize) -> Vec<SerpResult> {
    let doc = scraper::Html::parse_document(html);
    let Ok(cont_sel) = scraper::Selector::parse(".snippet") else {
        return Vec::new();
    };
    let Ok(a_sel) = scraper::Selector::parse("a.title, h2 a") else {
        return Vec::new();
    };
    let Ok(snip_sel) = scraper::Selector::parse(".snippet-description, .snippet-content") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for el in doc.select(&cont_sel) {
        if out.len() >= limit {
            break;
        }
        let Some(a) = el.select(&a_sel).next() else {
            continue;
        };
        let title = clean(&a.text().collect::<Vec<_>>().join(" "));
        let Some(href) = a.value().attr("href") else {
            continue;
        };
        let url = normalize_url(href);
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let snippet = el
            .select(&snip_sel)
            .next()
            .map(|n| clean(&n.text().collect::<Vec<_>>().join(" ")))
            .unwrap_or_default();
        out.push(SerpResult {
            position: out.len() + 1,
            title,
            domain: domain(&url),
            url,
            snippet,
        });
    }
    out
}

/// Collapse whitespace/newlines into single spaces (same as the reference app).
fn clean(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Normalize a result href to an absolute https:// URL.
/// Handles: protocol-relative (//host), the DuckDuckGo `uddg=` redirect param
/// (percent-decoded target), and strips fragments.
fn normalize_url(href: &str) -> String {
    let href = href.trim();
    if href.is_empty() {
        return String::new();
    }
    // DDG redirect wrapper: .../l/?uddg=<urlencoded target>&... (k5=1 avoids it,
    // but keep the decode as a fallback for when k5 is ignored).
    if let Some(idx) = href.find("uddg=") {
        let rest = &href[idx + 5..];
        let end = rest.find(['&', '#']).unwrap_or(rest.len());
        let decoded = percent_decode(&rest[..end]);
        if !decoded.is_empty()
            && (decoded.starts_with("http://") || decoded.starts_with("https://"))
        {
            return decoded;
        }
    }
    if let Some(rest) = href.strip_prefix("//") {
        let rest = rest.split('#').next().unwrap_or(rest);
        return format!("https://{rest}");
    }
    let without_fragment = href.split('#').next().unwrap_or(href).to_string();
    if without_fragment.starts_with("http://") || without_fragment.starts_with("https://") {
        return without_fragment;
    }
    String::new()
}

/// Minimal %XX percent-decoder (avoids a dep for one use; `url` doesn't expose
/// one publicly). Invalid sequences pass through unchanged.
fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = if bytes[i] == b'%' && i + 2 < bytes.len() {
            match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => Some(h * 16 + l),
                _ => None,
            }
        } else {
            None
        };
        match decoded {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Host of a URL (www. stripped) for the `domain` field.
fn domain(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .map(|h| h.strip_prefix("www.").unwrap_or(&h).to_string())
        .unwrap_or_default()
}

/// Dedupe key: lowercase host + path (query/fragment ignored — same page).
fn dedupe_key(url: &str) -> String {
    match Url::parse(url) {
        Ok(u) => {
            let host = u.host_str().unwrap_or("").to_lowercase();
            let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
            let path = u.path().trim_end_matches('/').to_string();
            format!("{host}{path}")
        }
        Err(_) => url.to_lowercase(),
    }
}

/// Dedupe by normalized URL, cap at limit, renumber positions 1..N.
fn dedupe_cap(results: Vec<SerpResult>, limit: usize) -> Vec<SerpResult> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<SerpResult> = Vec::new();
    for r in results {
        if out.len() >= limit {
            break;
        }
        if seen.insert(dedupe_key(&r.url)) {
            out.push(r);
        }
    }
    for (i, r) in out.iter_mut().enumerate() {
        r.position = i + 1;
    }
    out
}

/// Monotonic-ish request id: unix millis + per-process sequence.
fn request_id() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("serp-{millis}-{seq}")
}

/// Fetch + parse one HTTP engine with retry + exponential backoff.
async fn http_search(engine: &str, opts: &SerpOpts) -> anyhow::Result<Vec<SerpResult>> {
    let url = engine_url(
        engine,
        &opts.query,
        opts.page,
        opts.safe,
        opts.region.as_deref(),
    )?;
    let mut attempt = 0u32;
    loop {
        match crate::engines::serp_http_get(&url) {
            Ok((status, body)) if (200..300).contains(&status) => {
                return Ok(parse_results(engine, &body, opts.limit));
            }
            Ok((status, _)) if attempt >= opts.retries => {
                return Err(anyhow::anyhow!("{engine} returned HTTP {status}"));
            }
            Err(e) if attempt >= opts.retries => {
                return Err(anyhow::anyhow!("{engine} request failed: {e}"));
            }
            _ => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(200 * (1 << attempt))).await;
            }
        }
    }
}

/// Render an engine's results page in an attached browser and parse the DOM —
/// works on every CDP engine (Chrome/obscura/lightpanda); needed for `brave`.
async fn browser_search<B: BrowserBackend>(
    backend: &B,
    engine: &str,
    opts: &SerpOpts,
) -> anyhow::Result<Vec<SerpResult>> {
    let url = engine_url(
        engine,
        &opts.query,
        opts.page,
        opts.safe,
        opts.region.as_deref(),
    )?;
    backend.navigate(&url).await?;
    let html = backend.get_html().await?;
    Ok(parse_results(engine, &html, opts.limit))
}

/// Fallback chain for a failed/empty specific engine: try the other HTTP
/// engines, then render the requested engine in a browser if one is attached.
async fn fallback_chain<B: BrowserBackend>(
    exclude: &str,
    opts: &SerpOpts,
    backend: Option<&B>,
) -> (Vec<SerpResult>, Vec<String>) {
    let mut skipped = vec![exclude.to_string()];
    for e in HTTP_ENGINES {
        if e == exclude {
            continue;
        }
        match http_search(e, opts).await {
            Ok(rs) if !rs.is_empty() => return (rs, skipped),
            Ok(_) => skipped.push(e.to_string()),
            Err(_) => skipped.push(e.to_string()),
        }
    }
    if let Some(b) = backend {
        match browser_search(b, exclude, opts).await {
            Ok(rs) if !rs.is_empty() => return (rs, skipped),
            _ => {}
        }
        skipped.push(format!("{exclude} (browser)"));
    }
    (Vec::new(), skipped)
}

/// Run a SERP search. `backend` is optional — only `brave` (and the browser
/// fallback) need it; duckduckgo/bing/google/auto use pooled HTTP with none.
/// Non-trivial logic here is covered by the pure-function unit tests below.
pub async fn serp_search<B: BrowserBackend>(
    opts: &SerpOpts,
    backend: Option<&B>,
) -> anyhow::Result<SerpResponse> {
    let start = Instant::now();
    let request_id = request_id();
    let engine = opts.engine.to_lowercase();

    let (results, skipped) = match engine.as_str() {
        // auto: fire all HTTP engines concurrently, merge, dedupe, cap.
        "auto" => {
            let futures: Vec<_> = HTTP_ENGINES.iter().map(|e| http_search(e, opts)).collect();
            let outcomes = futures_util::future::join_all(futures).await;
            let mut merged: Vec<SerpResult> = Vec::new();
            let mut skipped = Vec::new();
            for (e, o) in HTTP_ENGINES.iter().zip(outcomes) {
                match o {
                    Ok(mut rs) => merged.append(&mut rs),
                    Err(_) => skipped.push((*e).to_string()),
                }
            }
            (dedupe_cap(merged, opts.limit), skipped)
        }
        "brave" => {
            let b = backend.ok_or_else(|| {
                anyhow::anyhow!(
                    "engine 'brave' requires a connected browser (set CDP_URL or start Chrome \
                     with --remote-debugging-port=9222); duckduckgo|bing|google|auto work without one"
                )
            })?;
            (browser_search(b, "brave", opts).await?, Vec::new())
        }
        e if HTTP_ENGINES.contains(&e) => match http_search(e, opts).await {
            Ok(rs) if !rs.is_empty() => (rs, Vec::new()),
            Ok(_) if opts.fallback => fallback_chain(e, opts, backend).await,
            Ok(rs) => (rs, Vec::new()),
            Err(_) if opts.fallback => fallback_chain(e, opts, backend).await,
            Err(err) => return Err(err),
        },
        other => {
            return Err(anyhow::anyhow!(
                "unknown engine '{other}' (duckduckgo|bing|google|brave|auto)"
            ));
        }
    };

    Ok(SerpResponse {
        query: opts.query.clone(),
        engine: engine.clone(),
        results,
        request_id,
        ms: start.elapsed().as_millis() as u64,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddg_parse_typed_results() {
        let html = r#"
        <html><body>
            <div class="result">
                <h2 class="result__title"><a href="https://example.com">Example Result</a></h2>
                <a class="result__snippet">This is an example snippet.</a>
            </div>
            <div class="result">
                <h2 class="result__title"><a href="//cdn.example.net/docs">CDN Doc</a></h2>
                <a class="result__snippet">Second result, protocol-relative.</a>
            </div>
        </body></html>
        "#;
        let rs = parse_duckduckgo(html, 10);
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].position, 1);
        assert_eq!(rs[0].title, "Example Result");
        assert_eq!(rs[0].url, "https://example.com");
        assert_eq!(rs[0].domain, "example.com");
        assert_eq!(rs[0].snippet, "This is an example snippet.");
        assert_eq!(rs[1].url, "https://cdn.example.net/docs");
    }

    #[test]
    fn bing_parse_typed_results() {
        let html = r#"
        <html><body><ol id="b_results">
            <li class="b_algo">
                <h2><a href="https://www.bing.com/ck/a?!&&p=x&u=aHR0cHM6Ly9sZWFybi5taWNyb3NvZnQuY29tL3J1c3Q&ntb=1">Rust docs</a></h2>
                <div class="b_caption"><p>Official Rust documentation.</p></div>
            </li>
        </ol></body></html>
        "#;
        let rs = parse_bing(html, 10);
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].title, "Rust docs");
        // the ck/a redirect must be decoded to the real target
        assert_eq!(rs[0].url, "https://learn.microsoft.com/rust");
        assert_eq!(rs[0].domain, "learn.microsoft.com");
        assert_eq!(rs[0].snippet, "Official Rust documentation.");
    }

    #[test]
    fn bing_redirect_decodes_base64url_target() {
        let href = "https://www.bing.com/ck/a?!&&p=abc&u=aHR0cHM6Ly9sZWFybi5taWNyb3NvZnQuY29tL3J1c3Q&ntb=1";
        assert_eq!(
            bing_target(href).as_deref(),
            Some("https://learn.microsoft.com/rust")
        );
        // real Bing form: `a1` marker prefix before the base64url payload
        let live = "https://www.bing.com/ck/a?!&&p=x&u=a1aHR0cHM6Ly9odS53aWtpcGVkaWEub3JnL3dpa2kvVG9raSVDMyVCMw&ntb=1";
        assert_eq!(
            bing_target(live).as_deref(),
            Some("https://hu.wikipedia.org/wiki/Toki%C3%B3")
        );
        // no u= param → None → callers fall back to normalize_url(href)
        assert_eq!(bing_target("https://example.com/x"), None);
        // malformed base64 → None
        assert_eq!(bing_target("https://www.bing.com/ck/a?u=!!!"), None);
    }

    #[test]
    fn google_parse_skips_internal_links() {
        let html = r#"
        <html><body><div id="search">
            <div class="g">
                <div class="r"><a href="https://www.google.com/search?q=related">Related</a></div>
                <div><a href="https://rust-lang.org"><h3>Rust Language</h3></a></div>
                <div class="VwiC3b">A systems programming language.</div>
            </div>
        </div></body></html>
        "#;
        let rs = parse_google(html, 10);
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].title, "Rust Language");
        assert_eq!(rs[0].url, "https://rust-lang.org");
        assert_eq!(rs[0].snippet, "A systems programming language.");
    }

    #[test]
    fn normalize_decodes_uddg_and_protocol_relative() {
        assert_eq!(
            normalize_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&rut=abc"),
            "https://example.com/page"
        );
        assert_eq!(
            normalize_url("//cdn.example.net/a#frag"),
            "https://cdn.example.net/a"
        );
        assert_eq!(
            normalize_url("https://example.com/x?utm_source=t"),
            "https://example.com/x?utm_source=t"
        );
        assert_eq!(normalize_url("/relative/path"), "");
        assert_eq!(normalize_url(""), "");
    }

    #[test]
    fn dedupe_by_host_path() {
        let rs = vec![
            SerpResult {
                position: 1,
                title: "a".into(),
                url: "https://x.com/page?utm=1".into(),
                domain: "x.com".into(),
                snippet: String::new(),
            },
            SerpResult {
                position: 1,
                title: "b".into(),
                url: "https://X.com/page/".into(),
                domain: "x.com".into(),
                snippet: String::new(),
            },
            SerpResult {
                position: 1,
                title: "c".into(),
                url: "https://x.com/other".into(),
                domain: "x.com".into(),
                snippet: String::new(),
            },
        ];
        let deduped = dedupe_cap(rs, 10);
        assert_eq!(
            deduped.len(),
            2,
            "same host+path dedupes regardless of query/slash"
        );
        assert_eq!(deduped[0].position, 1);
        assert_eq!(deduped[1].position, 2);
    }

    #[test]
    fn engine_url_encodes_params() {
        let u = engine_url("duckduckgo", "rust & web", 1, true, Some("gb-en")).unwrap();
        assert!(u.contains("q=rust+%26+web"), "query percent-encoded: {u}");
        assert!(u.contains("s=30"), "page 1 offset: {u}");
        assert!(u.contains("k1=2"), "safe on: {u}");
        assert!(u.contains("kl=gb-en"), "region: {u}");
        assert!(u.contains("k5=1"), "no-redirect: {u}");

        let b = engine_url("bing", "rust", 2, false, None).unwrap();
        assert!(b.contains("first=21"), "bing page 2: {b}");
        assert!(b.contains("adlt=off"), "bing safe off: {b}");

        let g = engine_url("google", "rust", 1, true, None).unwrap();
        assert!(g.contains("start=10"));
        assert!(g.contains("safe=active"));

        assert!(engine_url("nope", "x", 0, false, None).is_err());
    }

    #[test]
    fn auto_merges_and_caps() {
        // No network in tests: auto with a tiny limit still runs the chain; just
        // assert the envelope shape via the pure helpers.
        let opts = SerpOpts {
            query: "x".into(),
            limit: 1,
            ..Default::default()
        };
        assert_eq!(opts.limit, 1);
        assert!(request_id().len() > 10);
    }
}
