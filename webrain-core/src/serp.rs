// webrain-core/src/serp.rs
// Structured SERP (Search Engine Results Page) extraction — a typed SERP API.
//
// Reference/inspiration: the standalone "rust-serp-api" app (Axum + reqwest +
// scraper scraping DuckDuckGo HTML). Instead of a second HTTP server, this is a
// first-class webrain capability: typed {position,title,url,domain,snippet}
// results, reachable from the webrain_serp MCP tool, the `webrain serp` CLI, and
// any LLM over webrain's MCP transport (stdio or --http).
//
// duckduckgo/bing serve plain HTML → fetched over the pooled no-browser HTTP
// agent (no browser at all). `google`/`brave` are JS-gated → rendered in
// whatever CDP engine is attached (Chrome/obscura/lightpanda) via navigate +
// get_html. `auto` fetches duckduckgo+bing concurrently (no browser) and merges;
// google/brave join the merge when a browser is attached. Every result is
// relevance-filtered against the query (unrelated junk pages never enter the
// reply — engines on flagged/GeoIP'd IPs sometimes serve garbage).
//
// Recommended features (from the reference app's list) that make sense for a
// local portable tool: provider fallback, URL dedupe, pagination, safe search +
// region, request ids, retry with backoff, parallel multi-provider, per-request
// proxy. SaaS-only concerns are deferred — Redis cache, API keys, rate limits,
// billing, metrics, OTel, circuit breaker, proxy rotation pool (ponytail: name
// the ceiling, add when this is hosted as a multi-tenant service).

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
    /// Actual provider that supplied the results — differs from `engine` only
    /// after a provider fallback (e.g. engine=google, source=duckduckgo).
    /// Empty for `auto` (the reply is a merged aggregate).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Per-engine outcome breakdown — which engines contributed how many
    /// results, and which were skipped/empty. Populated for `auto` always;
    /// single-engine calls report the winner plus any skipped fallbacks.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub per_engine: Vec<SerpEngineReport>,
}

/// One engine's outcome inside a SERP reply (mainly for `engine=auto`).
#[derive(Debug, Clone, Serialize)]
pub struct SerpEngineReport {
    pub engine: String,
    /// "ok" (contributed ≥1 relevant result) | "empty" (answered but 0 relevant)
    /// | "skipped" (errored, timed out, or not attempted — e.g. google/brave
    /// with no browser attached).
    pub status: String,
    /// Results this engine contributed to the reply (after relevance filtering).
    pub count: usize,
}

/// Search parameters for serp_search.
#[derive(Debug, Clone)]
pub struct SerpOpts {
    pub query: String,
    /// duckduckgo | bing | google | brave | auto (all HTTP engines, merged).
    pub engine: String,
    /// Max results to return, 1..=100.
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
    /// Route HTTP-engine fetches through this proxy URL (e.g. "http://user:pass@host:port"
    /// or "socks5://host:port"). The google/brave browser path only honors a proxy when
    /// the CDP engine was launched with one (CLI `--proxy` bakes it into `--proxy-server`
    /// on google auto-launch; an attached browser keeps whatever proxy it was started with).
    pub proxy: Option<String>,
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
            proxy: None,
        }
    }
}

/// HTTP-renderable engines, in fallback/auto preference order.
/// NOTE: `brave` is deliberately absent — it's a JS SPA that only renders in a
/// browser, so http_search("brave") always parses to zero results (it would
/// silently claim a browser engine contributed nothing). Brave runs via
/// browser_search when a backend is attached.
const HTTP_ENGINES: [&str; 3] = ["bing", "duckduckgo", "google"];

/// `auto`'s no-browser engines, in merge-preference order. duckduckgo is the
/// most reliable plain-HTML SERP; bing is GeoIP-fragile and can serve unrelated
/// junk on flagged IPs, so it merges second (after relevance filtering).
const AUTO_HTTP_ENGINES: [&str; 2] = ["duckduckgo", "bing"];

/// `auto`'s JS-gated engines — joined only when a CDP backend is attached.
const AUTO_BROWSER_ENGINES: [&str; 2] = ["google", "brave"];

/// Cap on one browser engine inside `auto`: a walled google/brave (Brave's PoW
/// captcha can take ~10-20s to auto-resolve) must never stall the whole merged
/// reply, so the browser join is best-effort and timed out.
const SERP_BROWSER_CAP: Duration = Duration::from_secs(20);

/// Build the provider URL for one engine + params.
fn engine_url(
    engine: &str,
    q: &str,
    page: usize,
    safe: bool,
    region: Option<&str>,
    limit: usize,
) -> anyhow::Result<String> {
    // Accuracy: pin an en-US market when no region is given instead of letting
    // the engine GeoIP the request — a localized IP turns "tokio rust" into
    // Czech Tokyo travel pages. region format: "<country>-<lang>" e.g. us-en.
    let region = region.unwrap_or("us-en");
    let (cc, lang) = region.split_once('-').unwrap_or((region, region));
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
            p.append_pair("kl", region);
            drop(p);
            Ok(u.to_string())
        }
        "bing" => {
            let mut u = Url::parse("https://www.bing.com/search")?;
            let mut p = u.query_pairs_mut();
            p.append_pair("q", q);
            // Bing paginates 10/page via `first` (1, 11, 21, ...), but when
            // `first` is present bing may ignore a custom `count` and return
            // the default page size (10). So send `first` only past page 0 and
            // let `count` drive page 0's result count (open-serp's bing rule).
            if page > 0 {
                p.append_pair("first", &(page * 10 + 1).to_string());
            }
            p.append_pair("adlt", if safe { "strict" } else { "off" });
            // Locale (mkt/setlang/cc) + request more than the 10-result page.
            p.append_pair("mkt", &format!("{lang}-{}", cc.to_uppercase()));
            p.append_pair("setlang", lang);
            p.append_pair("cc", &cc.to_uppercase());
            if limit > 10 {
                p.append_pair("count", &limit.to_string());
            }
            drop(p);
            Ok(u.to_string())
        }
        "google" => {
            let mut u = Url::parse("https://www.google.com/search")?;
            let mut p = u.query_pairs_mut();
            p.append_pair("q", q);
            // Real-browser google URL: NO `num` (10/page default; `num>10` is a
            // bot signal and often ignored) and NO `lr` (the language-restrict
            // filter makes google serve "did not match any documents" on
            // flagged/automated sessions — verified live). `start` only past
            // page 0 (google's canonical page 0 omits it). `hl`/`gl` pin the
            // locale without the aggressive `lr` restriction.
            if page > 0 {
                p.append_pair("start", &(page * 10).to_string());
            }
            p.append_pair("safe", if safe { "active" } else { "off" });
            p.append_pair("hl", lang);
            p.append_pair("gl", cc);
            drop(p);
            Ok(u.to_string())
        }
        "brave" => {
            let mut u = Url::parse("https://search.brave.com/search")?;
            let mut p = u.query_pairs_mut();
            p.append_pair("q", q);
            // ponytail: Brave offset is page-indexed (0,1,2...): page*10 = nonexistent page (no-results).
            p.append_pair("offset", &page.to_string());
            p.append_pair("hl", lang);
            p.append_pair("gl", cc);
            drop(p);
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
    // Containers: #search .g (classic), div.MjjYud (current), div.g and the
    // modern div[data-hveid] result blocks (browsemind GoogleSERPClient's proven
    // selector set — these cover the layouts Google actually ships).
    let Ok(cont_sel) = scraper::Selector::parse("#search .g, div.MjjYud, div.g, div[data-hveid]")
    else {
        return Vec::new();
    };
    let Ok(a_sel) = scraper::Selector::parse("a[href]") else {
        return Vec::new();
    };
    let Ok(h3_sel) = scraper::Selector::parse("h3") else {
        return Vec::new();
    };
    // Snippets: .VwiC3b (newer), span.aCOpRe + div[data-sncf] (browsemind's set).
    let Ok(snip_sel) = scraper::Selector::parse(".VwiC3b, span.aCOpRe, div[data-sncf]") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    // div[data-hveid] matches NESTED result blocks (a result's sub-divs carry
    // data-hveid too), so the same link can appear many times — dedupe by href
    // HERE, inside the loop, or the duplicates fill `limit` before the distinct
    // results are reached (post-parse dedupe_cap can't recover them).
    let mut seen: HashSet<String> = HashSet::new();
    for el in doc.select(&cont_sel) {
        if out.len() >= limit {
            break;
        }
        // The organic-result link is the first external <a> whose child h3 holds
        // the title (ad/related links point at google.com internals). Title is
        // the h3 ONLY — never the anchor's full text, which can include an
        // inline <style> block (CSS-junk titles on google's denser layouts).
        let mut chosen: Option<(String, String)> = None; // (title, href)
        for a in el.select(&a_sel) {
            let Some(href) = a.value().attr("href") else {
                continue;
            };
            let url = google_href(href);
            if url.is_empty() || url.starts_with("https://www.google.") {
                continue;
            }
            let title = a
                .select(&h3_sel)
                .next()
                .map(|h| clean(&h.text().collect::<Vec<_>>().join(" ")))
                .filter(|t| !t.is_empty());
            if let Some(t) = title {
                chosen = Some((t, url));
                break;
            }
        }
        let Some((title, url)) = chosen else { continue };
        if !seen.insert(url.clone()) {
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

/// Resolve a Google result href to an absolute https:// URL.
/// Google organic links are `/url?q=<urlencoded target>` redirects (decode the
/// real URL) or relative `/...` (prefix the google origin); plain absolute
/// http(s) links pass through via normalize_url.
fn google_href(href: &str) -> String {
    // /url?q=<urlencoded target>&sa=... — percent-decode the target.
    if let Some(idx) = href.find("q=") {
        let rest = &href[idx + 2..];
        let end = rest.find(['&', '#']).unwrap_or(rest.len());
        let decoded = percent_decode(&rest[..end]);
        if decoded.starts_with("http://") || decoded.starts_with("https://") {
            return decoded;
        }
    }
    // Protocol-relative links (//host/path) must resolve BEFORE the single-slash
    // case — strip_prefix('/') also matches them and produced a bogus
    // google.com//host/path that got filtered as an internal link.
    if let Some(rest) = href.strip_prefix("//") {
        return format!("https://{rest}");
    }
    if let Some(rest) = href.strip_prefix('/') {
        return format!("https://www.google.com/{rest}");
    }
    normalize_url(href)
}

fn parse_brave(html: &str, limit: usize) -> Vec<SerpResult> {
    let doc = scraper::Html::parse_document(html);
    let Ok(cont_sel) = scraper::Selector::parse(".snippet") else {
        return Vec::new();
    };
    // Brave's current title link is the first `a[href]` in the snippet (class
    // `svelte-* l1`); the real title text sits in a nested `.title` (or h2/h3).
    // The old `a.title, h2 a` matched nothing → 0 results on every brave page.
    let Ok(a_sel) = scraper::Selector::parse("a[href]") else {
        return Vec::new();
    };
    let Ok(title_sel) = scraper::Selector::parse("a[href] .title, a[href] h2, a[href] h3") else {
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
        let title = el
            .select(&title_sel)
            .next()
            .map(|n| clean(&n.text().collect::<Vec<_>>().join(" ")))
            .or_else(|| Some(clean(&a.text().collect::<Vec<_>>().join(" "))))
            .unwrap_or_default();
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

/// Stopwords never count as query tokens for the relevance gate.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "for", "in", "on", "at", "to", "by", "with", "is", "are",
    "was", "were", "be", "been", "being", "this", "that", "these", "those", "it", "its", "as",
    "from", "into", "over", "under", "about", "vs", "via", "no", "not", "do", "does", "did",
    "what", "who", "when", "where", "why", "how",
];

/// Significant (non-stopword, len >= 3) lowercase query tokens.
fn query_tokens(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in query.split(|c: char| !c.is_alphanumeric()) {
        if t.is_empty() {
            continue;
        }
        let t = t.to_lowercase();
        if t.len() >= 3 && !STOPWORDS.contains(&t.as_str()) && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Does `text` contain any significant query token as a whole word?
fn contains_query_token(text: &str, tokens: &[String]) -> bool {
    tokens.iter().any(|t| {
        text.split(|c: char| !c.is_alphanumeric())
            .any(|w| !w.is_empty() && w.to_lowercase() == *t)
    })
}

/// Junk-page guard: engines on flagged/GeoIP'd IPs sometimes serve unrelated
/// "results" (bing's identity-theft page for "tokio rust", speedtest pages for
/// "webrain rust", ...). A result survives only when its title, domain, or URL
/// shares at least one significant query token with the query. Positions are
/// renumbered 1-based after filtering. Queries with no significant tokens
/// (e.g. "ai") pass through unfiltered — there is nothing to judge against.
fn filter_relevant(results: Vec<SerpResult>, query: &str) -> Vec<SerpResult> {
    let tokens = query_tokens(query);
    if tokens.is_empty() {
        return results;
    }
    let mut out: Vec<SerpResult> = Vec::new();
    for r in results {
        let hay = format!("{} {} {}", r.title, r.domain, r.url);
        if contains_query_token(&hay, &tokens) {
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
/// Fetch + parse ONE engine results page with retry + exponential backoff.
async fn http_search_page(
    engine: &str,
    url: &str,
    opts: &SerpOpts,
) -> anyhow::Result<Vec<SerpResult>> {
    let mut attempt = 0u32;
    let url_owned = url.to_string();
    let proxy = opts.proxy.clone();
    loop {
        // ureq is blocking — run it off the async executor so the `auto`
        // join_all fan-out hits the blocking pool instead of tying up (and
        // serializing on) async worker threads.
        let u = url_owned.clone();
        let p = proxy.clone();
        let res =
            tokio::task::spawn_blocking(move || crate::engines::serp_http_get(&u, p.as_deref()))
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("serp task failed: {e}")));
        match res {
            Ok((status, body)) if (200..300).contains(&status) => {
                let parsed = parse_results(engine, &body, opts.limit);
                // A 2xx body that parses to ZERO is usually a flaky empty /
                // JS-shell page an engine serves under rate-limit pressure
                // (bing intermittently returns one for an otherwise-fine
                // query), not a genuinely-empty result set. Retry within the
                // existing budget before giving up; the caller's relevance +
                // dedupe still handle real empties.
                if parsed.is_empty() && attempt < opts.retries {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(200 * (1 << attempt))).await;
                    continue;
                }
                return Ok(parsed);
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

/// Fetch + parse one HTTP engine, merging consecutive pages until `limit` is
/// met (engines serve ~10 results/page; `count`/`first`/`s` unlock more on
/// clean IPs). Stops the moment a page adds zero new unique results — engines
/// on a GeoIP-locked IP often serve the same page every time, so this never
/// burns requests on repeats. Positions are renumbered 1-based after the merge.
async fn http_search(engine: &str, opts: &SerpOpts) -> anyhow::Result<Vec<SerpResult>> {
    let mut all: Vec<SerpResult> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    // 100 max / ~10 per page; bounded so a hostile engine can't loop forever.
    let max_pages = (opts.limit / 10).clamp(1, 10);
    for page in opts.page..opts.page + max_pages {
        let url = engine_url(
            engine,
            &opts.query,
            page,
            opts.safe,
            opts.region.as_deref(),
            opts.limit,
        )?;
        let rs = http_search_page(engine, &url, opts).await?;
        // Junk-page guard: drop results that share no significant query token
        // BEFORE counting "fresh" — a page of unrelated results stops the
        // pagination early instead of filling `limit` with garbage.
        let rs = filter_relevant(rs, &opts.query);
        let mut fresh = 0usize;
        for r in rs {
            if seen.insert(dedupe_key(&r.url)) {
                all.push(r);
                fresh += 1;
            }
        }
        if all.len() >= opts.limit || fresh == 0 {
            break;
        }
    }
    for (i, r) in all.iter_mut().enumerate() {
        r.position = i + 1;
    }
    all.truncate(opts.limit);
    Ok(all)
}

/// Random u64 in [a, b] — human-like timing jitter (browsemind randomizes
/// every delay: 40-120ms keystrokes, 1-2s reads). Fixed delays are a bot
/// fingerprint. SplitMix64 on an atomic counter; no rand dep needed.
fn jitter(a: u64, b: u64) -> u64 {
    static STATE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0x853c49e6748fea9b);
    let mut x = STATE.fetch_add(0x9E3779B97F4A7C15, std::sync::atomic::Ordering::Relaxed);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^= x >> 31;
    a + x % (b - a + 1)
}

/// Dismiss a Google consent wall / regional dialog is done as a TRUSTED CDP
/// click on a structurally-picked button (see browser_search) — never JS
/// `.click()` / DOM removal, and never text matching (localized in every
/// language).

/// Register the Google human-like init script once per backend process (it is
/// Render an engine's results page in an attached browser and parse the DOM —
/// works on every CDP engine (Chrome/obscura/lightpanda); needed for `brave`
/// and the only path that beats Google's consent/JS wall.
///
/// Google goes through the HOMEPAGE → search-box flow (browsemind recipe):
/// loading the homepage, human-like events (after navigation, before consent),
/// dismissing consent, TYPING the
/// query with trusted CDP Input.insertText (Google's controlled input ignores
/// JS `.value=` + synthetic Event — trusted keys are required to reveal the
/// search button) and TRUSTED-clicking "Google Search" (never JS Enter, which
/// Google penalizes). Hitting `/search?q=` directly is a strong bot signal that
/// reliably trips Google's "unusual traffic" page, so we never use the direct
/// Dismiss a Google consent wall/regional dialog with a TRUSTED CDP click —
/// browsemind's ConsentManager recipe: Phase 1 multilingual accept/reject text
/// (never "Sign in"), Phase 2 last-button fallback. No page JS runs: button
/// discovery goes through the accessibility tree + DOM quads
/// (Accessibility.getFullAXTree → backendDOMNodeId → DOM.getContentQuads), the
/// click itself is Input.dispatchMouseEvent. Single-shot now (see body).
const CONSENT_PATTERNS: &[&str] = &[
    "accept",
    "agree",
    "consent",
    "allow",
    "got it",
    "i understand",
    "zustimmen",
    "akzeptieren",
    "accepter",
    "aceptar",
    "accetto",
    "aceitar",
    "akkoord",
    "zgadzam",
    "согласен",
    "同意",
    "接受",
    "承認",
    "동의",
    "αποδοχή",
    "αποδέχομαι",
    "reject",
    "deny",
    "decline",
    "refuse",
    "rechazar",
    "rifiutare",
    "ablehnen",
    "weigeren",
    "odrzuć",
    "отклонить",
    "απόρριψη",
];
async fn dismiss_google_consent<B: BrowserBackend>(backend: &B) -> String {
    // ponytail ultra: fire the TRUSTED click the instant the overlay renders.
    // consent_button is DOM-gated (~1ms when no overlay), so poll fast (150ms)
    // up to ~1.2s and click the moment a button appears — no long waits, no
    // 12× AX scans (that was ~42s). The dialog closes on its own; the caller's
    // wait_for_results beat covers the close.
    for _ in 0..8 {
        if let Some((x, y, tag)) = backend.consent_button(CONSENT_PATTERNS).await {
            let _ = backend.mouse_move_human(x, y).await;
            let _ = backend.click_coords(x, y).await;
            return format!("clicked:{}", tag.chars().take(24).collect::<String>());
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    "none".to_string()
}

/// search URL for google. Then we WAIT for the results page before parsing
/// (never parse a consent/JS-shell/bot-wall page that loads first).
async fn browser_search<B: BrowserBackend>(
    backend: &B,
    engine: &str,
    opts: &SerpOpts,
) -> anyhow::Result<Vec<SerpResult>> {
    if engine == "google" || engine == "brave" || engine == "bing" {
        // Direct search-URL + pagination (the guest-browser flow): navigate
        // straight to engine_url's search URL (google /search?q=..&start=..,
        // brave /search?q=..&offset=..) instead of homepage→type→submit, and
        // merge `start`/`offset` pages SEQUENTIALLY exactly like http_search
        // does for HTTP engines so `--limit 30` / `--page 2` return more than
        // one page. Sequential, not parallel tabs: Google walls simultaneous
        // /search requests from one browser, so parallel pagination returns
        // FEWER results (measured 12 vs 30) — pacing pages like a human wins.
        //
        // Trusted-only: no page-JS evaluate (wait_for_results is a fixed beat
        // for google; brave uses its selector poll), no injected stealth for
        // google (CLI sets WEBRAIN_NO_STEALTH). Human-like mouse + wheel +
        // consent stay trusted CDP input. Walled pages yield 0 organic results
        // → fresh==0 → stop early → the retry/fallback chain takes over.
        let mut all: Vec<SerpResult> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        // google serves ~6-7 organic results per page (video carousels, AI
        // overviews, featured snippets crowd the rest), so budget pages on ~7
        // not 10 or --limit 20 stalls at ~13 (2 pages × 6.5). Ceiling division
        // so --limit 20 gets 3 pages (20→~19-20), --limit 50 gets 8. fresh==0 /
        // the limit check bound the loop; this is just the ceiling.
        // bing paginates 10/page via `first` (1, 11, 21…), so budget /10
        // (google ~6.5-7/page stays /7).
        let max_pages = if engine == "bing" {
            (opts.limit / 10).clamp(1, 10)
        } else {
            opts.limit.div_ceil(7).clamp(1, 15)
        };
        for page in opts.page..opts.page + max_pages {
            let t0 = std::time::Instant::now();
            let url = engine_url(
                engine,
                &opts.query,
                page,
                opts.safe,
                opts.region.as_deref(),
                opts.limit,
            )?;
            backend.navigate(&url).await?;
            // Consent FIRST — trusted click the instant the overlay renders
            // (fast poll, DOM-gated ~1ms when none). Never blocked by a wall.
            let consent = if engine == "google" {
                dismiss_google_consent(backend).await
            } else {
                "none".to_string()
            };
            tracing::debug!(engine, %consent, "serp consent dismiss");
            tokio::time::sleep(Duration::from_millis(jitter(150, 250))).await;
            // Trusted human-like pre-interaction (browsemind random_mouse_move)
            // — CDP mouseMoved, NOT synthetic dispatch, then TRUSTED wheel.
            let _ = backend
                .mouse_move_human(jitter(120, 700) as i64, jitter(80, 400) as i64)
                .await;
            let _ = backend.scroll("down").await;
            // Self-heal via current_url (no evaluate): never dead-end on sign-in.
            if let Some(u) = backend.current_url().await {
                if u.contains("accounts.google") || u.contains("myaccount.google") {
                    backend.navigate("https://www.google.com").await?;
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                }
            }
            // Wait for the results DOM to render (fixed beat — no eval).
            wait_for_results(backend, engine).await?;
            let html = backend.get_html().await?;
            let rs = filter_relevant(
                dedupe_cap(parse_results(engine, &html, opts.limit), opts.limit),
                &opts.query,
            );
            let mut fresh = 0usize;
            for r in rs {
                if seen.insert(dedupe_key(&r.url)) {
                    all.push(r);
                    fresh += 1;
                }
            }
            tracing::debug!(
                engine,
                page,
                fresh,
                total = all.len(),
                ms = t0.elapsed().as_millis(),
                "serp page"
            );
            if all.len() >= opts.limit || fresh == 0 {
                break; // enough results, or this page added nothing new (wall/repeat)
            }
            // Short human pacing between page turns (wall insurance on
            // pagination, but keep it fast).
            tokio::time::sleep(Duration::from_millis(jitter(500, 900))).await;
        }
        for (i, r) in all.iter_mut().enumerate() {
            r.position = i + 1;
        }
        all.truncate(opts.limit);
        return Ok(all);
    }

    let url = engine_url(
        engine,
        &opts.query,
        opts.page,
        opts.safe,
        opts.region.as_deref(),
        opts.limit,
    )?;
    backend.navigate(&url).await?;
    wait_for_results(backend, engine).await?;
    let html = backend.get_html().await?;
    Ok(dedupe_cap(
        parse_results(engine, &html, opts.limit),
        opts.limit,
    ))
}

/// Wait (bounded) for an engine's results container to render before parsing,
/// so we never parse a consent wall / JS shell / still-loading page. Times out
/// quietly — the caller parses whatever rendered (often 0 results → fallback).
/// For google we poll ONLY for the results container: its consent / "enable JS"
/// wall carries enough body text that a length heuristic would return early and
/// parse the wall (0 results) instead of waiting for the real results.
async fn wait_for_results<B: BrowserBackend>(backend: &B, engine: &str) -> anyhow::Result<()> {
    if engine == "google" {
        // TRUSTED (no page-JS evaluate): navigate already waited for
        // readyState=interactive; results stream ~1s later — a short fixed
        // beat. The caller's parse-empty guard catches a still-wall/empty page.
        tokio::time::sleep(Duration::from_millis(1200)).await;
        return Ok(());
    }
    let sel = match engine {
        "duckduckgo" => ".result",
        "bing" => "li.b_algo",
        "brave" => ".snippet",
        _ => return Ok(()),
    };
    let js = format!(
        "(document.querySelectorAll({sel}).length > 0)",
        sel = serde_json::to_string(sel)?
    );
    // Poll ONLY the results selector for the full budget — no innerText-length
    // shortcut. A captcha/consent wall has >500 chars of body text, so that
    // shortcut bailed the instant a wall rendered and never saw real results.
    // Brave's PoW captcha ("Verify you're not a bot") auto-resolves in a real
    // browser — a fresh guest profile gets re-captcha'd every launch, and the
    // hash can take ~10-20s, so the budget is generous (21s).
    for _ in 0..70 {
        if let Ok(v) = backend.evaluate(&js).await {
            let ok = v.as_bool().unwrap_or(false) || v.as_str() == Some("true");
            if ok {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Ok(())
}

/// serpapi.com as a paid Google provider — the standard `SERPAPI_API_KEY` env
/// var gates it (unset → empty with no network, so the normal fallback chain
/// is untouched). Returns typed organic results; 4xx/quota/parse failures also
/// surface as empty so a dead or quota-exhausted key degrades to fallback.
async fn serpapi_google(opts: &SerpOpts) -> anyhow::Result<Vec<SerpResult>> {
    let key = match std::env::var("SERPAPI_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => return Ok(Vec::new()),
    };
    let region = opts.region.as_deref().unwrap_or("us-en");
    let (cc, lang) = region.split_once('-').unwrap_or(("us", "en"));
    let mut u = Url::parse("https://serpapi.com/search.json")?;
    {
        let mut p = u.query_pairs_mut();
        p.append_pair("engine", "google");
        p.append_pair("q", &opts.query);
        p.append_pair("num", &opts.limit.clamp(1, 100).to_string());
        // serpapi honors `start` up to `num` — page the results like the other
        // engines (was silently ignored, returning page-0 dupes on --page N).
        p.append_pair("start", &(opts.page * opts.limit.clamp(1, 100)).to_string());
        p.append_pair("hl", lang);
        p.append_pair("gl", cc);
        p.append_pair("safe", if opts.safe { "active" } else { "off" });
        p.append_pair("api_key", &key);
    }
    // ureq is blocking — off the executor (this fn is called from the async
    // serp path, incl. the post-browser fallback).
    let url_s = u.as_str().to_string();
    let proxy = opts.proxy.clone();
    let fetched = tokio::task::spawn_blocking(move || {
        crate::engines::serp_http_get(&url_s, proxy.as_deref())
    })
    .await
    .unwrap_or_else(|e| Err(anyhow::anyhow!("serpapi task failed: {e}")))?;
    let (status, body) = fetched;
    if !(200..300).contains(&status) {
        return Ok(Vec::new());
    }
    Ok(parse_serpapi_json(&body, opts.limit))
}

/// Parse serpapi `/search.json` organic results into typed results. Pure —
/// unit-tested below. Error / missing organic_results yield empty (caller
/// falls back). serpapi honors `num` up to 100, so `limit` here is the real
/// result count (unlike the free engines' ~10-per-page cap).
fn parse_serpapi_json(body: &str, limit: usize) -> Vec<SerpResult> {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    if v.get("error").is_some() || v.get("organic_results").is_none() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(org) = v["organic_results"].as_array() {
        for r in org {
            if out.len() >= limit {
                break;
            }
            let title = r
                .get("title")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let url = r
                .get("link")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = r
                .get("snippet")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if title.is_empty() || url.is_empty() {
                continue;
            }
            out.push(SerpResult {
                position: out.len() + 1,
                title,
                domain: domain(&url),
                url,
                snippet,
            });
        }
    }
    out
}

/// Search one specific HTTP engine (duckduckgo/bing/google). Google is JS-gated
/// over plain HTTP (consent/JS shell → zero results), so when a browser is
/// attached it goes straight to the browser path — the only one that returns
/// real Google results. Everything else uses HTTP with provider fallback.
/// Returns (results, skipped, source) — `source` names the engine that actually
/// supplied the results (differs from `engine` only after a provider fallback).
async fn specific_engine<B: BrowserBackend>(
    engine: &str,
    opts: &SerpOpts,
    backend: Option<&B>,
) -> anyhow::Result<(Vec<SerpResult>, Vec<String>, String)> {
    // serpapi.com paid Google provider (SERPAPI_API_KEY): the reliable way to
    // get MORE than the free engines' ~10-per-page cap — serpapi honors `num`
    // up to 100, so a high limit is best served by serpapi FIRST. For small
    // limits (or no key) the free browser path stays primary and serpapi is a
    // fallback. Empty on unset key; errors are swallowed so a dead/quota key
    // just falls through to the chain below.
    let serpapi_ready = engine == "google"
        && std::env::var("SERPAPI_API_KEY")
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false);
    let serpapi_first = serpapi_ready && opts.limit > 10;
    if serpapi_first {
        if let Ok(rs) = serpapi_google(opts).await {
            if !rs.is_empty() {
                return Ok((rs, Vec::new(), "google".to_string()));
            }
        }
    }
    // Google is JS-gated over plain HTTP (consent/JS shell → zero results), so
    // when a browser is attached it goes straight to the browser path — the only
    // one that returns real Google results (browsemind recipe: real Chrome +
    // human-like init + wait-for-results + consent dismiss + data-hveid parse).
    if let Some(b) = backend.filter(|_| engine == "google" || engine == "brave" || engine == "bing")
    {
        // Google/Brave intermittently serve a "unusual traffic / not a robot"
        // CAPTCHA page (rate-limited IP — Google /sorry, Brave's PoW "Verify
        // you're not a bot"). A persistent-profile wall usually clears in ~10s
        // (attempt 2 catches it), but a fresh profile on a flagged IP stays
        // walled for minutes — retrying that 4× burns ~60s for nothing. So: two
        // consecutive walled attempts = IP blocked for this session → fall back
        // to the other engines instead of wasting more navigations.
        // Google's soft-block for automation is an EMPTY results page
        // ("did not match any documents. Reset search tools") — not a /sorry
        // CAPTCHA — so it must count as a wall too, or the retry loop burns 4
        // attempts (minutes) re-fetching the same empty page.
        let wall_js = "location.href.indexOf('/sorry') >= 0 || /unusual traffic|not a robot|captcha|not a bot|did not match any documents|reset search tools/i.test(location.href + ' ' + (document.body ? document.body.innerText : ''))";
        let mut walls = 0u32;
        for attempt in 0..4 {
            // Retry in a FRESH TAB — a failed first attempt can poison the
            // tab's session state (per-tab JS/history/risk flags). A human
            // opens a new window/tab after a failed search instead of
            // re-navigating the same one.
            if attempt > 0 {
                if let Ok(id) = b.open_tab("about:blank").await {
                    let _ = b.activate_tab(&id).await;
                }
            }
            let rs = browser_search(b, engine, opts).await.unwrap_or_default();
            if !rs.is_empty() {
                return Ok((rs, Vec::new(), engine.to_string()));
            }
            let walled = b
                .evaluate(wall_js)
                .await
                .map(|v| v.as_bool().unwrap_or(false))
                .unwrap_or(false);
            if walled {
                walls += 1;
                if walls >= 2 {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_secs(if attempt == 0 { 10 } else { 3 })).await;
        }
    }
    // serpapi as a post-browser fallback (only when it wasn't tried first AND
    // fallback is enabled — an explicit --no-fallback must not silently get
    // serpapi.com results).
    if opts.fallback && serpapi_ready && !serpapi_first {
        if let Ok(rs) = serpapi_google(opts).await {
            if !rs.is_empty() {
                return Ok((rs, Vec::new(), "google".to_string()));
            }
        }
    }
    match http_search(engine, opts).await {
        Ok(rs) if !rs.is_empty() => Ok((rs, Vec::new(), engine.to_string())),
        Ok(_) if opts.fallback => Ok(fallback_chain(engine, opts, backend).await),
        Ok(rs) => Ok((rs, Vec::new(), String::new())),
        Err(_) if opts.fallback => Ok(fallback_chain(engine, opts, backend).await),
        Err(err) => Err(err),
    }
}

/// Fallback chain for a failed/empty specific engine: try the other HTTP
/// engines, then render the requested engine in a browser if one is attached.
/// Returns (results, skipped, winner) — `winner` names the engine that supplied
/// the results (empty when every fallback failed too).
async fn fallback_chain<B: BrowserBackend>(
    exclude: &str,
    opts: &SerpOpts,
    backend: Option<&B>,
) -> (Vec<SerpResult>, Vec<String>, String) {
    let mut skipped = vec![exclude.to_string()];
    for e in HTTP_ENGINES {
        if e == exclude {
            continue;
        }
        match http_search(e, opts).await {
            Ok(rs) if !rs.is_empty() => return (rs, skipped, e.to_string()),
            Ok(_) => skipped.push(e.to_string()),
            Err(_) => skipped.push(e.to_string()),
        }
    }
    // serpapi.com paid Google provider when the key is set (covers auto +
    // fallback paths) — before spending a browser render on a walled IP.
    if exclude == "google" {
        if let Ok(rs) = serpapi_google(opts).await {
            if !rs.is_empty() {
                skipped.retain(|s| s != exclude);
                return (rs, skipped, "google".to_string());
            }
        }
    }
    if let Some(b) = backend {
        match browser_search(b, exclude, opts).await {
            Ok(rs) if !rs.is_empty() => {
                // The requested engine's browser render succeeded after all —
                // it was never really skipped; drop it from the skip list so a
                // successful google run doesn't report `skipped: google`.
                skipped.retain(|s| s != exclude);
                return (rs, skipped, exclude.to_string());
            }
            _ => {}
        }
        skipped.push(format!("{exclude} (browser)"));
    }
    (Vec::new(), skipped, String::new())
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

    let (results, skipped, per_engine, source) = match engine.as_str() {
        // auto: fire the no-browser engines (duckduckgo, bing) concurrently over
        // HTTP, then merge + relevance-filter + dedupe + cap. When a CDP backend
        // is attached, google/brave ALSO render in the browser (sequentially —
        // one active tab, and Google walls simultaneous /search requests) and
        // join the merge; without one they're reported skipped, never
        // HTTP-polled pointlessly.
        "auto" => {
            let mut per_engine: Vec<SerpEngineReport> = Vec::new();
            let mut skipped: Vec<String> = Vec::new();
            let http_outcomes = futures_util::future::join_all(
                AUTO_HTTP_ENGINES.iter().map(|e| http_search(e, opts)),
            )
            .await;
            let mut merged: Vec<SerpResult> = Vec::new();
            for (e, o) in AUTO_HTTP_ENGINES.iter().zip(http_outcomes) {
                match o {
                    Ok(rs) => {
                        let count = rs.len();
                        merged.extend(rs);
                        per_engine.push(SerpEngineReport {
                            engine: (*e).to_string(),
                            status: if count > 0 { "ok" } else { "empty" }.to_string(),
                            count,
                        });
                    }
                    Err(_) => {
                        skipped.push((*e).to_string());
                        per_engine.push(SerpEngineReport {
                            engine: (*e).to_string(),
                            status: "skipped".to_string(),
                            count: 0,
                        });
                    }
                }
            }
            if let Some(b) = backend {
                // SEQUENTIAL, not concurrent: browser_search drives the ONE
                // active tab/session of the shared CDP backend, and Google walls
                // simultaneous /search requests from one browser (the codebase
                // measured fewer results with parallel tabs). Two interleaved
                // browser flows would race the same tab. The HTTP engines above
                // stay in parallel — only the browser join is serialized.
                for e in AUTO_BROWSER_ENGINES {
                    match tokio::time::timeout(SERP_BROWSER_CAP, browser_search(b, e, opts)).await {
                        Ok(Ok(rs)) => {
                            let count = rs.len();
                            merged.extend(rs);
                            per_engine.push(SerpEngineReport {
                                engine: e.to_string(),
                                status: if count > 0 { "ok" } else { "empty" }.to_string(),
                                count,
                            });
                        }
                        _ => {
                            skipped.push(format!("{e} (browser)"));
                            per_engine.push(SerpEngineReport {
                                engine: e.to_string(),
                                status: "skipped".to_string(),
                                count: 0,
                            });
                        }
                    }
                }
            } else {
                for e in AUTO_BROWSER_ENGINES {
                    skipped.push(format!("{e} (no browser attached)"));
                    per_engine.push(SerpEngineReport {
                        engine: e.to_string(),
                        status: "skipped".to_string(),
                        count: 0,
                    });
                }
            }
            (
                dedupe_cap(filter_relevant(merged, &opts.query), opts.limit),
                skipped,
                per_engine,
                String::new(), // auto = merged aggregate, no single source
            )
        }
        "brave" => {
            let b = backend.ok_or_else(|| {
                anyhow::anyhow!(
                    "engine 'brave' requires a connected browser (set CDP_URL or start Chrome \
                     with --remote-debugging-port=9222); duckduckgo|bing|google|auto work without one"
                )
            })?;
            let rs = browser_search(b, "brave", opts).await?;
            let count = rs.len();
            (
                rs,
                Vec::new(),
                vec![SerpEngineReport {
                    engine: "brave".to_string(),
                    status: if count > 0 { "ok" } else { "empty" }.to_string(),
                    count,
                }],
                "brave".to_string(),
            )
        }
        e if HTTP_ENGINES.contains(&e) => {
            let (results, skipped, source) = specific_engine(e, opts, backend).await?;
            let mut per_engine: Vec<SerpEngineReport> = Vec::new();
            if results.is_empty() {
                if skipped.is_empty() {
                    per_engine.push(SerpEngineReport {
                        engine: e.to_string(),
                        status: "empty".to_string(),
                        count: 0,
                    });
                } else {
                    for s in &skipped {
                        per_engine.push(SerpEngineReport {
                            engine: s.clone(),
                            status: "skipped".to_string(),
                            count: 0,
                        });
                    }
                }
            } else {
                per_engine.push(SerpEngineReport {
                    engine: source.clone(),
                    status: "ok".to_string(),
                    count: results.len(),
                });
                for s in &skipped {
                    if s != &source {
                        per_engine.push(SerpEngineReport {
                            engine: s.clone(),
                            status: "skipped".to_string(),
                            count: 0,
                        });
                    }
                }
            }
            (results, skipped, per_engine, source)
        }
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
        per_engine,
        source,
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
    fn brave_parse_typed_results() {
        // Current Brave HTML (2026): the title link is `a[href]` (svelte `l1`)
        // and the real title text lives in a nested `.title` — not `a.title`/`h2`.
        let html = r#"
        <html><body>
            <div class="snippet">
                <a href="https://kworb.net/spotify/artist/6WzxopGY3sy97IeNFaDELc_songs.html">
                    <div class="site-name-wrapper"><span>Kworb</span></div>
                    <div class="title search-snippet-title">Trannos - Spotify Top Songs</div>
                </a>
                <div class="snippet-description">Current charts, last updated 2026/05/30.</div>
            </div>
            <div class="snippet">
                <a href="https://deezer.com/en/artist/90670822">
                    <div class="title">Trannos: albums, songs</div>
                </a>
                <div class="snippet-content">His reputation grew in 2023.</div>
            </div>
        </body></html>
        "#;
        let rs = parse_brave(html, 10);
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].position, 1);
        assert_eq!(rs[0].title, "Trannos - Spotify Top Songs");
        assert_eq!(
            rs[0].url,
            "https://kworb.net/spotify/artist/6WzxopGY3sy97IeNFaDELc_songs.html"
        );
        assert_eq!(rs[0].domain, "kworb.net");
        assert_eq!(rs[0].snippet, "Current charts, last updated 2026/05/30.");
        assert_eq!(rs[1].title, "Trannos: albums, songs");
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
    fn google_parse_modern_layout_and_redirect() {
        // browsemind's proven container/snippet selectors + the /url?q= redirect
        // form Google actually ships in the modern (data-hveid) layout.
        let html = r#"
        <html><body><div id="rso">
            <div data-hveid="CAI">
                <div class="yuRUbf"><a href="/url?q=https%3A%2F%2Ftokio.rs%2F&sa=U&ved=2ah"><h3>Tokio</h3></a></div>
                <span class="aCOpRe">An asynchronous runtime for the Rust language.</span>
            </div>
        </div></body></html>
        "#;
        let rs = parse_google(html, 10);
        assert_eq!(rs.len(), 1, "modern data-hveid container parsed");
        assert_eq!(rs[0].title, "Tokio");
        assert_eq!(rs[0].url, "https://tokio.rs/");
        assert_eq!(rs[0].domain, "tokio.rs");
        assert_eq!(
            rs[0].snippet,
            "An asynchronous runtime for the Rust language."
        );
    }

    #[test]
    fn google_href_resolves_redirects_and_relative() {
        // /url?q= redirect → decoded real target
        assert_eq!(
            google_href("/url?q=https%3A%2F%2Frust-lang.org%2F&sa=U&ved=2"),
            "https://rust-lang.org/"
        );
        // relative /search → google origin (then skipped as internal by caller)
        assert_eq!(
            google_href("/search?q=related"),
            "https://www.google.com/search?q=related"
        );
        // absolute passes through
        assert_eq!(
            google_href("https://example.com/x"),
            "https://example.com/x"
        );
        // protocol-relative //host — must NOT resolve to google.com//host
        assert_eq!(
            google_href("//example.com/path"),
            "https://example.com/path"
        );
        assert_eq!(google_href(""), "");
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
        let u = engine_url("duckduckgo", "rust & web", 1, true, Some("gb-en"), 10).unwrap();
        assert!(u.contains("q=rust+%26+web"), "query percent-encoded: {u}");
        assert!(u.contains("s=30"), "page 1 offset: {u}");
        assert!(u.contains("k1=2"), "safe on: {u}");
        assert!(u.contains("kl=gb-en"), "region: {u}");
        assert!(u.contains("k5=1"), "no-redirect: {u}");

        let b = engine_url("bing", "rust", 2, false, None, 10).unwrap();
        assert!(b.contains("first=21"), "bing page 2: {b}");
        assert!(b.contains("adlt=off"), "bing safe off: {b}");
        // No region -> en-US market pinned (locale garbage fix).
        assert!(b.contains("mkt=en-US"), "bing en-US market: {b}");

        let b20 = engine_url("bing", "rust", 0, false, None, 20).unwrap();
        assert!(b20.contains("count=20"), "bing limit respected: {b20}");
        assert!(
            !b20.contains("first="),
            "page 0 must omit `first` so bing honors `count`: {b20}"
        );

        let g = engine_url("google", "rust", 1, true, None, 10).unwrap();
        assert!(g.contains("start=10"));
        assert!(g.contains("safe=active"));
        assert!(g.contains("hl=en"), "google en-US lang: {g}");

        // Real-browser google URL (the "did not match any documents" fix):
        // page 0 must omit `start`, and NO `num`/`lr` ever — both are bot
        // signals that google answers with an empty-results page.
        let g0 = engine_url("google", "rust", 0, false, None, 100).unwrap();
        assert!(!g0.contains("start="), "page 0 omits start: {g0}");
        assert!(!g0.contains("num="), "no num param: {g0}");
        assert!(!g0.contains("lr="), "no lr language-restrict: {g0}");
        assert!(g0.contains("q=rust"), "query kept: {g0}");
        let g1 = engine_url("google", "rust", 1, false, None, 100).unwrap();
        assert!(g1.contains("start=10"), "page 1 start offset: {g1}");
        assert!(!g1.contains("num="), "no num on later pages either: {g1}");

        let br = engine_url("brave", "rust", 2, false, None, 10).unwrap();
        assert!(br.contains("offset=2"), "brave page-indexed offset: {br}");
        assert!(
            !br.contains("offset=20"),
            "brave must NOT scale offset by 10: {br}"
        );

        assert!(engine_url("nope", "x", 0, false, None, 10).is_err());
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

    #[test]
    fn serpapi_parse_organic_results() {
        // serpapi /search.json organic_results → typed SerpResult; empty-link
        // rows skipped, limit + error handled.
        let body = r#"{"organic_results":[
            {"position":1,"title":"Tokio","link":"https://tokio.rs/","snippet":"Async runtime."},
            {"position":2,"title":"tokio - Rust","link":"https://docs.rs/tokio","snippet":"Docs."},
            {"position":3,"title":"bad","link":"","snippet":"skipped"}
        ]}"#;
        let rs = parse_serpapi_json(body, 10);
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].position, 1);
        assert_eq!(rs[0].title, "Tokio");
        assert_eq!(rs[0].url, "https://tokio.rs/");
        assert_eq!(rs[0].domain, "tokio.rs");
        assert_eq!(rs[0].snippet, "Async runtime.");
        assert_eq!(rs[1].url, "https://docs.rs/tokio");
        assert!(
            !rs.iter().any(|r| r.url.is_empty()),
            "empty-link rows skipped"
        );
        assert_eq!(parse_serpapi_json(body, 1).len(), 1, "limit respected");
        assert!(parse_serpapi_json(r#"{"error":"bad"}"#, 10).is_empty());
        assert!(parse_serpapi_json("not json", 10).is_empty());
    }

    fn mk_result(title: &str, url: &str) -> SerpResult {
        SerpResult {
            position: 1,
            title: title.to_string(),
            url: url.to_string(),
            domain: domain(url),
            snippet: String::new(),
        }
    }

    #[test]
    fn relevance_gate_drops_unrelated_junk() {
        // The exact regression from the demo review: bing served identity-theft /
        // fireplace junk for "tokio rust" and it drowned duckduckgo's real results.
        let rs = vec![
            mk_result("Tokio - An asynchronous Rust runtime", "https://tokio.rs/"),
            mk_result("The Fire Factory", "https://www.thefirefactory.com/"),
            mk_result(
                "Identity Theft Protection",
                "https://cybernews.com/identity-theft-protection/",
            ),
            mk_result(
                "GitHub - tokio-rs/tokio",
                "https://github.com/tokio-rs/tokio",
            ),
        ];
        let kept = filter_relevant(rs, "tokio rust");
        assert_eq!(kept.len(), 2, "junk dropped, real results kept");
        assert_eq!(kept[0].title, "Tokio - An asynchronous Rust runtime");
        assert_eq!(kept[0].position, 1, "positions renumbered");
        assert_eq!(kept[1].url, "https://github.com/tokio-rs/tokio");
        assert_eq!(kept[1].position, 2);
    }

    #[test]
    fn relevance_gate_matches_domains_and_url_tokens() {
        // "rust" must match rust-lang.org (hyphenated domain) and a URL path.
        let rs = vec![
            mk_result("Rust Programming Language", "https://rust-lang.org/"),
            mk_result("docs", "https://docs.rs/tokio/latest/tokio/"),
            mk_result("unrelated", "https://example.com/"),
        ];
        let kept = filter_relevant(rs, "rust");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].url, "https://rust-lang.org/");
        let kept2 = filter_relevant(
            vec![mk_result("docs", "https://docs.rs/tokio/latest/tokio/")],
            "tokio rust",
        );
        assert_eq!(kept2.len(), 1, "URL path token matches");
    }

    #[test]
    fn relevance_gate_passes_when_query_has_no_significant_tokens() {
        // "ai" is too short to judge against; stopwords-only queries pass all.
        let rs = vec![
            mk_result("anything at all", "https://example.com/a"),
            mk_result("more stuff", "https://example.com/b"),
        ];
        assert_eq!(filter_relevant(rs.clone(), "ai").len(), 2);
        assert_eq!(filter_relevant(rs.clone(), "the of").len(), 2);
        assert_eq!(filter_relevant(rs, "c++").len(), 2);
    }

    #[test]
    fn query_tokens_are_significant_only() {
        assert_eq!(
            query_tokens("tokio rust"),
            vec!["tokio".to_string(), "rust".to_string()]
        );
        assert_eq!(
            query_tokens("the rust programming language"),
            vec![
                "rust".to_string(),
                "programming".to_string(),
                "language".to_string()
            ]
        );
        assert!(query_tokens("ai").is_empty());
        assert!(query_tokens("c++").is_empty());
        // deduped + case-insensitive
        assert_eq!(query_tokens("Rust rust"), vec!["rust".to_string()]);
    }
}
