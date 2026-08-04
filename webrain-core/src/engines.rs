use crate::backends::cdp::CdpBackend;
use crate::browser::{BrowserBackend, PageResult};
use serde_json::{json, Value};

/// PixelRAG-style tile capture: split the page into a grid of screenshot tiles
/// so a vision model can read specific regions (tables/charts/layout survive).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TileShot {
    pub index: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub png_b64: String,
}

pub struct TileEngine {
    pub tile_width: f64,
    pub tile_height: f64,
    pub max_tiles: usize,
}

impl TileEngine {
    pub fn new(tile_width: f64, tile_height: f64, max_tiles: usize) -> Self {
        Self {
            tile_width,
            tile_height,
            max_tiles,
        }
    }

    /// Split the current page into tiles. Call navigate/snapshot first; does NOT navigate.
    /// ponytail: CDP clip per tile (no image decode); cap total tiles at max_tiles.
    pub async fn tile(&self, backend: &impl BrowserBackend) -> anyhow::Result<Vec<TileShot>> {
        let size: Value = backend
            .evaluate("[document.documentElement.scrollWidth, document.documentElement.scrollHeight]")
            .await?;
        let page_w = size[0].as_f64().unwrap_or(1280.0).max(1.0);
        let page_h = size[1].as_f64().unwrap_or(800.0).max(1.0);

        let cols = ((page_w / self.tile_width).ceil() as usize).max(1);
        let rows = ((page_h / self.tile_height).ceil() as usize).max(1);

        let mut tiles = Vec::new();
        let mut idx = 0usize;
        'outer: for r in 0..rows {
            for c in 0..cols {
                if idx >= self.max_tiles {
                    break 'outer;
                }
                let x = c as f64 * self.tile_width;
                let y = r as f64 * self.tile_height;
                let w = self.tile_width.min(page_w - x);
                let h = self.tile_height.min(page_h - y);
                let png = backend.screenshot_clip(x, y, w, h).await?;
                tiles.push(TileShot {
                    index: idx,
                    x,
                    y,
                    width: w,
                    height: h,
                    png_b64: base64_encode(&png),
                });
                idx += 1;
            }
        }
        Ok(tiles)
    }
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    // ponytail: one self-check for the non-trivial base64 encoder.
    #[test]
    fn base64_roundtrip() {
        let s = b"hello";
        let encoded = super::base64_encode(s);
        assert_eq!(encoded, "aGVsbG8=");
        assert_eq!(super::base64_encode(b"a"), "YQ==");
        assert_eq!(super::base64_encode(b"ab"), "YWI=");
    }

    // ponytail: one check for the regex JS builder (builtin present, custom overrides).
    #[test]
    fn regex_js_has_builtins_and_overrides() {
        let js = super::build_regex_js(&[]);
        assert!(js.contains("\"label\":\"email\""));
        assert!(js.contains("document.documentElement.outerHTML"));
        let custom = vec![serde_json::json!({"label": "email", "re": "x@y"})];
        let js2 = super::build_regex_js(&custom);
        assert!(js2.contains("\"re\":\"x@y\""));
        assert!(!js2.contains(r"[A-Za-z0-9._%+-]+@"));
    }

    // ponytail: one check for extended extract_js — nested, base_fields, list.
    #[test]
    fn extract_js_nested_and_base_fields() {        let base_fields = vec![serde_json::json!({"name":"url","attribute":"href"})];
        let fields = vec![serde_json::json!({
            "name": "title", "selector": "h2", "type": "text"
        }), serde_json::json!({
            "name": "details", "selector": "div.details", "type": "nested",
            "fields": [
                {"name":"brand","selector":"span.brand","type":"text"},
                {"name":"model","selector":"span.model","type":"text"}
            ]
        })];
        let js = super::build_extract_js("div.product", &base_fields, &fields);
        // base_fields: url from container href
        assert!(js.contains("\"url\": (el.getAttribute('href') ?? null)"));
        // nested: {details: {brand:..., model:...}}
        assert!(js.contains("\"details\": (function(){const c=el.querySelector('div.details');"));
        assert!(js.contains("\"brand\":"));
        assert!(js.contains("\"model\":"));
        // list type
        let list_fields = vec![serde_json::json!({
            "name":"features","selector":"ul.features li","type":"list"
        })];
        let js2 = super::build_extract_js("div.product", &[], &list_fields);
        assert!(js2.contains("Array.from(el.querySelectorAll('ul.features li')).map(c=>"));
    }

    // ponytail: one check for adaptive extract — exact path + relocation fallback present.
    #[test]
    fn adaptive_js_has_exact_and_relocation_paths() {
        let fields = vec![serde_json::json!({
            "name": "title", "selector": "h2", "type": "text"
        }), serde_json::json!({
            "name": "price", "selector": "span.price", "type": "text"
        })];
        let js = super::build_adaptive_extract_js("div.product", &[], &fields);
        // exact path first: querySelectorAll('div.product')
        assert!(js.contains("document.querySelectorAll('div.product')"));
        assert!(js.contains("if (exact.length) return JSON.stringify(exact);"));
        // relocation probe embeds field selectors and requires >= 2 hits
        assert!(js.contains("FIELD_SELS = ['h2', 'span.price']"));
        assert!(js.contains("hits >= 2"));
        assert!(js.contains("c.contains(d)"));
        // exact builder stays untouched (no fallback)
        let exact = super::build_extract_js("div.product", &[], &fields);
        assert!(!exact.contains("FIELD_SELS"));
        assert!(exact.contains("JSON.stringify(Array.from"));
    }

    // ponytail: one check for BM25 — query "rust" ranks the rust doc first.
    #[test]
    fn bm25_ranks_relevant_first() {
        let items = vec![
            "A guide to cooking pasta with tomatoes".to_string(),
            "Rust programming language async runtime documentation".to_string(),
            "Python web framework tutorial".to_string(),
        ];
        let r = super::bm25_filter(&items, "rust documentation", 2);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0]["index"], 1); // rust doc scores highest
    }

    // ponytail: one check for the Chrome header contract — sec-ch-ua GREASE
    // brand (v=24) + Chrome brand major agree with the UA, so the HTTP layer
    // looks internally consistent (obscura's identity-alignment point).
    #[test]
    fn chrome_headers_agree_with_ua() {
        assert!(super::CHROME_UA.contains("Chrome/145."));
        assert!(super::SEC_CH_UA.contains("\"Google Chrome\";v=\"145\""));
        assert!(super::SEC_CH_UA.contains("Not)A;Brand\";v=\"24\"")); // GREASE brand
        assert!(super::BROWSER_HEADERS
            .iter()
            .all(|(k, v)| !k.is_empty() && !v.is_empty()));
        assert!(super::SEC_CH_UA.contains("Chromium\";v=\"145\""));
    }

    // ponytail: one check for the spider allow/deny filter (branchy logic).
    // Sitemap parse is verified live — regex over simple XML, low risk.
    #[test]
    fn spider_filters_allow_deny() {
        let s = super::SpiderEngine::new(2, 10)
            .with_filters(vec!["/product/".to_string()], vec!["/cart".to_string(), "/login".to_string()]);
        assert!(s.url_ok("https://site.com/product/1"));
        assert!(!s.url_ok("https://site.com/about"));            // fails allow
        assert!(!s.url_ok("https://site.com/product/cart"));     // fails deny
        // empty allow = allow all; deny still prunes
        let s2 = super::SpiderEngine::new(2, 10).with_filters(vec![], vec!["/login".to_string()]);
        assert!(s2.url_ok("https://site.com/product/1"));
        assert!(!s2.url_ok("https://site.com/login"));
    }

    // ponytail: one check for AutoThrottle math — fast server speeds up, a block
    // doubles (capped at max), and the politeness floor is never undercut.
    #[test]
    fn autothrottle_speeds_up_and_backs_off() {
        let s = super::SpiderEngine::new(2, 10)
            .with_delay_ms(100)
            .with_autothrottle(true, 200, 5000);
        let mut d = std::collections::HashMap::new();
        // fast server (50ms latency): delay moves down toward ~50, floored at 100
        let d1 = s.throttle_tick(&mut d, "fast.com", 50, true);
        assert!(d1 >= 100 && d1 < 200);
        // slow-but-ok server (800ms): delay rises toward 800
        let d2 = s.throttle_tick(&mut d, "slow.com", 800, true);
        assert!(d2 >= 400 && d2 <= 800);
        // blocked: doubles each time, capped at max
        let d3 = s.throttle_tick(&mut d, "blocked.com", 5, false);
        let d4 = s.throttle_tick(&mut d, "blocked.com", 5, false);
        assert!(d4 >= d3 * 2 || d4 >= 200, "block should double: {d3} -> {d4}");
        let mut dmax = std::collections::HashMap::new();
        for _ in 0..10 { s.throttle_tick(&mut dmax, "capped.com", 1, false); }
        assert!(*dmax.get("capped.com").unwrap() <= 5000, "never exceeds max");
    }
}

/// Spider engine: BFS, DFS, or BestFirst crawler (crawl4ai deep-crawl strategies).
/// ponytail: BFS/DFS = VecDeque front/back; BestFirst = insertion-sorted by
/// keyword-relevance score (max at front). Domain + robots filter at enqueue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrawlStrategy {
    Bfs,
    Dfs,
    BestFirst,
}

#[derive(Debug, Clone)]
pub struct SpiderEngine {
    max_depth: usize,
    max_pages: usize,
    strategy: CrawlStrategy,
    same_domain: bool,
    allowed_domains: Vec<String>,
    /// Prefetch mode (crawl4ai `prefetch=True`): use the link-only `discover_links`
    /// fast path (no innerText extraction) and return URLs only.
    discover_only: bool,
    /// Honor robots.txt Disallow for the seed origin (crawl4ai `check_robots_txt`).
    respect_robots: bool,
    /// KeywordRelevanceScorer for BestFirst (crawl4ai): URL scored by keyword hits.
    keywords: Vec<String>,
    /// Only follow URLs matching ALL of these regexes (Scrapling LinkExtractor
    /// `allow` / spider-rs `whitelist_url`). Empty = allow all.
    allow: Vec<regex::Regex>,
    /// Skip URLs matching ANY of these regexes (Scrapling `deny` / spider-rs
    /// `blacklist_url`). Applied after allow.
    deny: Vec<regex::Regex>,
    /// Retry a failed page fetch up to N extra times (200ms backoff between).
    retry: u32,
    /// Polite delay between page fetches, ms (spider-rs `with_delay`).
    delay_ms: u64,
    /// Hard wall-clock cap on the whole crawl, seconds (spider-rs `crawl_timeout`).
    crawl_timeout_secs: Option<u64>,
    /// AutoThrottle (Scrapling AutoThrottle): per-domain adaptive delay tuned from
    /// observed latency. Speeds up on fast servers, doubles on a blocked/challenge
    /// response, capped at max. Floor = delay_ms (never undercut politeness).
    autothrottle: bool,
    autothrottle_start_delay_ms: u64,
    autothrottle_max_delay_ms: u64,
    /// Checkpoint/resume (Scrapling crawldir): persist {queue, seen} every N pages
    /// to this dir so a long crawl survives interruption and resumes from where it
    /// stopped. Deleted on a clean finish.
    crawldir: Option<std::path::PathBuf>,
    checkpoint_every: usize,
}

impl Default for SpiderEngine {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_pages: 20,
            strategy: CrawlStrategy::Bfs,
            same_domain: true,
            allowed_domains: vec![],
            discover_only: false,
            respect_robots: false,
            keywords: vec![],
            allow: vec![],
            deny: vec![],
            retry: 0,
            delay_ms: 0,
            crawl_timeout_secs: None,
            autothrottle: false,
            autothrottle_start_delay_ms: 200,
            autothrottle_max_delay_ms: 30_000,
            crawldir: None,
            checkpoint_every: 10,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpiderResult {
    pub page: PageResult,
    pub depth: usize,
    pub links: Vec<String>,
}

impl SpiderEngine {
    pub fn new(max_depth: usize, max_pages: usize) -> Self {
        Self { max_depth, max_pages, ..Default::default() }
    }

    pub fn with_strategy(mut self, s: CrawlStrategy) -> Self { self.strategy = s; self }
    pub fn with_same_domain(mut self, v: bool) -> Self { self.same_domain = v; self }
    pub fn with_allowed_domains(mut self, d: Vec<String>) -> Self { self.allowed_domains = d; self }
    pub fn with_discover_only(mut self, v: bool) -> Self { self.discover_only = v; self }
    pub fn with_respect_robots(mut self, v: bool) -> Self { self.respect_robots = v; self }
    pub fn with_keywords(mut self, k: Vec<String>) -> Self { self.keywords = k; self }
    /// Compile allow/deny regexes once. Invalid patterns are ignored (a bad deny
    /// regex must not silently let everything through — log it, skip the pattern).
    pub fn with_filters(mut self, allow: Vec<String>, deny: Vec<String>) -> Self {
        let compile = |pats: Vec<String>| -> Vec<regex::Regex> {
            pats.iter().filter_map(|p| match regex::Regex::new(p) {
                Ok(r) => Some(r),
                Err(e) => {
                    tracing::warn!("spider filter regex invalid '{p}': {e}");
                    None
                }
            }).collect()
        };
        self.allow = compile(allow);
        self.deny = compile(deny);
        self
    }
    pub fn with_retry(mut self, n: u32) -> Self { self.retry = n; self }
    pub fn with_delay_ms(mut self, ms: u64) -> Self { self.delay_ms = ms; self }
    /// 0 = no cap (Some(0) would make the deadline `now` and kill the crawl
    /// before the first page — treat it as "not set" at the shared entry point).
    pub fn with_crawl_timeout(mut self, secs: u64) -> Self {
        self.crawl_timeout_secs = if secs > 0 { Some(secs) } else { None };
        self
    }
    /// Enable Scrapling-style AutoThrottle. `floor_ms` = the spider's own
    /// `delay_ms` floor (never undercut). `max_ms` caps the adaptive delay.
    pub fn with_autothrottle(mut self, enabled: bool, start_ms: u64, max_ms: u64) -> Self {
        self.autothrottle = enabled;
        self.autothrottle_start_delay_ms = start_ms.max(self.delay_ms);
        self.autothrottle_max_delay_ms = max_ms.max(self.autothrottle_start_delay_ms);
        self
    }
    /// Enable checkpoint/resume: persist crawl state every `every` pages to `dir`.
    pub fn with_checkpoint(mut self, dir: String, every: usize) -> Self {
        self.crawldir = if dir.is_empty() { None } else { Some(std::path::PathBuf::from(dir)) };
        self.checkpoint_every = every.max(1);
        self
    }

    /// AutoThrottle: move the per-domain delay toward observed latency (fast
    /// servers speed up), or double it on a block (slow/hostile servers back off).
    /// Returns the delay to sleep before the next request to that domain.
    /// ponytail: `(cur + target) / 2` averaging like Scrapling; block doubling
    /// capped at max; delay never below the politeness floor.
    fn throttle_tick(
        &self,
        delays: &mut std::collections::HashMap<String, u64>,
        domain: &str,
        latency_ms: u64,
        ok: bool,
    ) -> u64 {
        if !self.autothrottle {
            return self.delay_ms;
        }
        let floor = self.delay_ms;
        let cur = *delays.get(domain).unwrap_or(&self.autothrottle_start_delay_ms);
        let new_delay = if ok {
            // Latency-driven: move halfway toward the server's real response time.
            let target = latency_ms.max(floor);
            let avg = (cur + target) / 2;
            avg.max(target.min(avg.max(target)))
                .min(self.autothrottle_max_delay_ms)
                .max(floor)
        } else {
            // Blocked/challenge: double (or wait longer if the site already
            // slowed us down). A block never speeds the crawl up.
            cur.saturating_mul(2).max(cur).min(self.autothrottle_max_delay_ms).max(floor)
        };
        delays.insert(domain.to_string(), new_delay);
        new_delay
    }

    /// Scrapling LinkExtractor filter: URL must match every `allow` (if any) and
    /// no `deny`. Empty allow = pass.
    fn url_ok(&self, url: &str) -> bool {
        if !self.allow.is_empty() && !self.allow.iter().all(|r| r.is_match(url)) {
            return false;
        }
        !self.deny.iter().any(|r| r.is_match(url))
    }

    /// Checkpoint file: {queue: [[url, depth]...], seen: [...]}. One JSON file,
    /// atomic-ish rewrite. ponytail: no serde for the queue — (String, usize)
    /// pairs serialize fine as arrays.
    fn checkpoint_path(&self) -> Option<std::path::PathBuf> {
        self.crawldir.as_ref().map(|d| d.join("checkpoint.json"))
    }

    fn save_checkpoint(
        &self,
        queue: &std::collections::VecDeque<(String, usize)>,
        visited: &std::collections::HashSet<String>,
    ) {
        let Some(path) = self.checkpoint_path() else { return };
        let q: Vec<Value> = queue.iter().map(|(u, d)| json!([u, d])).collect();
        let seen: Vec<String> = visited.iter().cloned().collect();
        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let data = json!({"queue": q, "seen": seen, "saved_at": saved_at});
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
        if let Ok(s) = serde_json::to_string(&data) {
            let _ = std::fs::write(&path, s);
        }
    }

    /// Restore {queue, seen} from checkpoint. Returns (queue, seen) or (empty,
    /// empty) when no checkpoint exists. ponytail: missing/corrupt = start fresh.
    fn load_checkpoint(
        &self,
    ) -> (std::collections::VecDeque<(String, usize)>, std::collections::HashSet<String>) {
        let mut queue = std::collections::VecDeque::new();
        let mut seen = std::collections::HashSet::new();
        let Some(path) = self.checkpoint_path() else { return (queue, seen) };
        let Ok(raw) = std::fs::read_to_string(path) else { return (queue, seen) };
        let Ok(data) = serde_json::from_str::<Value>(&raw) else { return (queue, seen) };
        if let Some(q) = data.get("queue").and_then(|v| v.as_array()) {
            for item in q {
                if let (Some(u), Some(d)) = (item[0].as_str(), item[1].as_u64()) {
                    queue.push_back((u.to_string(), d as usize));
                }
            }
        }
        if let Some(s) = data.get("seen").and_then(|v| v.as_array()) {
            for u in s.iter().filter_map(|x| x.as_str()) {
                seen.insert(u.to_string());
            }
        }
        (queue, seen)
    }

    fn delete_checkpoint(&self) {
        if let Some(path) = self.checkpoint_path() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Accept a URL for crawling based on domain filter.
    fn domain_ok(&self, href: &str, seed_host: &str) -> bool {
        if self.allowed_domains.is_empty() && !self.same_domain {
            return true;
        }
        // ponytail: url::Url parse; if the href is a relative path or malformed, let it in
        let host = if let Ok(u) = url::Url::parse(href) {
            u.host_str().unwrap_or("").to_lowercase()
        } else {
            return true; // relative URLs pass
        };
        if !self.allowed_domains.is_empty() {
            return self.allowed_domains.iter().any(|d| host == d.as_str() || host.ends_with(&format!(".{d}")));
        }
        host == seed_host || host.ends_with(&format!(".{seed_host}"))
    }

    pub async fn crawl(
        &self,
        browser: &impl BrowserBackend,
        seed_url: &str,
    ) -> Vec<SpiderResult> {
        use std::collections::{HashSet, VecDeque};

        let seed_host = url::Url::parse(seed_url)
            .ok()
            .and_then(|u| u.host_str().map(String::from))
            .unwrap_or_default();

        // robots.txt: fetch once for the seed origin, honor Disallow prefixes.
        // ponytail: single fetch, prefix match only; per-origin fetch for
        // multi-domain crawls when same_domain is disabled.
        let seed_origin = seed_url.split('/').take(3).collect::<Vec<_>>().join("/");
        let disallowed: Vec<String> = if self.respect_robots {
            ureq::get(&format!("{seed_origin}/robots.txt"))
                .call()
                .ok()
                .and_then(|r| r.into_body().read_to_string().ok())
                .map(|s| {
                    s.lines()
                        .filter_map(|l| {
                            l.trim()
                                .to_lowercase()
                                .strip_prefix("disallow:")
                                .map(|p| p.trim().to_string())
                        })
                        .filter(|p| !p.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            vec![]
        };
        let robots_ok = |link: &str| {
            if disallowed.is_empty() {
                return true;
            }
            let path = link
                .splitn(3, '/')
                .nth(2)
                .map(|p| p.to_lowercase())
                .unwrap_or_default();
            !disallowed.iter().any(|p| path.starts_with(p))
        };

        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        let mut results: Vec<SpiderResult> = Vec::new();
        let crawl_deadline = self.crawl_timeout_secs.map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
        // AutoThrottle per-domain delays (learned during this crawl, not persisted).
        let mut throttle: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

        // Checkpoint/resume: restore {queue, seen} from a previous interrupted run.
        if self.crawldir.is_some() {
            let (q, s) = self.load_checkpoint();
            queue = q;
            visited = s;
        }
        if queue.is_empty() {
            queue.push_back((seed_url.to_string(), 0));
            visited.insert(seed_url.to_string());
        }
        let mut since_checkpoint = 0usize;

        while let Some((url, depth)) = self.pop(&mut queue) {
            if results.len() >= self.max_pages {
                break;
            }
            if let Some(deadline) = crawl_deadline {
                if std::time::Instant::now() >= deadline {
                    break; // wall-clock cap (spider-rs crawl_timeout) — stop the crawl.
                }
            }

            let start = std::time::Instant::now();

            // AutoThrottle: sleep the learned delay for this domain before
            // fetching (floor = delay_ms; starts at autothrottle_start_delay_ms).
            let domain = url.split('/').nth(2).unwrap_or("").to_string();
            let pre_delay = if self.autothrottle {
                throttle.get(&domain).copied().unwrap_or(self.autothrottle_start_delay_ms).max(self.delay_ms)
            } else {
                self.delay_ms
            };
            if pre_delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(pre_delay)).await;
            }

            // ponytail: discover_only uses the link-only fast path (no innerText,
            // no full-load fallback) — that's crawl4ai prefetch / "no navigation".
            let (page_result, links): (PageResult, Vec<String>) = if self.discover_only {
                match browser.discover_links(&url).await {
                    Ok(links) => (
                        PageResult {
                            url: url.clone(),
                            title: None,
                            content: None,
                            screenshot_b64: None,
                            error: None,
                            duration_ms: start.elapsed().as_millis() as u64,
                        },
                        links,
                    ),
                    Err(e) => {
                        results.push(SpiderResult {
                            page: PageResult {
                                url: url.clone(),
                                title: None,
                                content: None,
                                screenshot_b64: None,
                                error: Some(e.to_string()),
                                duration_ms: start.elapsed().as_millis() as u64,
                            },
                            depth,
                            links: vec![],
                        });
                        continue;
                    }
                }
            } else {
                // retry (spider-rs with_retry): re-fetch up to N times on error.
                let mut page_result: PageResult = PageResult {
                    url: url.clone(),
                    title: None,
                    content: None,
                    screenshot_b64: None,
                    error: Some("unreached".into()),
                    duration_ms: 0,
                };
                let mut attempts = 0;
                loop {
                    match browser.navigate(&url).await {
                        Ok(state) => {
                            page_result = PageResult {
                                url: url.clone(),
                                title: Some(state.title.clone()),
                                content: Some(state.text),
                                screenshot_b64: None,
                                error: None,
                                duration_ms: start.elapsed().as_millis() as u64,
                            };
                            break;
                        }
                        Err(e) => {
                            attempts += 1;
                            if attempts > self.retry {
                                page_result.error = Some(e.to_string());
                                page_result.duration_ms = start.elapsed().as_millis() as u64;
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        }
                    }
                }
                if page_result.error.is_some() {
                    results.push(SpiderResult {
                        page: page_result,
                        depth,
                        links: vec![],
                    });
                    continue;
                }
                // ponytail: SPA/VitePress render links after initial DOM — short settle.
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                let links = match browser
                    .evaluate(
                        "Array.from(document.querySelectorAll('a[href]')).map(a => a.href)",
                    )
                    .await
                {
                    Ok(v) => {
                        let mut hrefs = vec![];
                        if let Some(arr) = v.as_array() {
                            for item in arr {
                                if let Some(s) = item.as_str() {
                                    hrefs.push(s.to_string());
                                }
                            }
                        }
                        hrefs
                    }
                    Err(_) => vec![],
                };
                (page_result, links)
            };

            results.push(SpiderResult {
                page: page_result,
                depth,
                links: links.clone(),
            });

            if depth < self.max_depth {
                for link in &links {
                    if !visited.contains(link)
                        && self.domain_ok(link, &seed_host)
                        && self.url_ok(link)
                        && robots_ok(link)
                    {
                        visited.insert(link.clone());
                        self.push_link(&mut queue, link.clone(), depth + 1);
                    }
                }
            }

            // AutoThrottle: feed the finished request back. Blocked = the page
            // errored OR carried a challenge (non-2xx / gated) → double the delay.
            if self.autothrottle {
                let blocked = results.last().map(|r| r.page.error.is_some()).unwrap_or(false);
                let _ = self.throttle_tick(
                    &mut throttle,
                    &domain,
                    start.elapsed().as_millis() as u64,
                    !blocked,
                );
            } else if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }

            // Checkpoint: persist {queue, seen} every N pages so a long crawl
            // survives interruption and resumes from here (Scrapling crawldir).
            since_checkpoint += 1;
            if self.crawldir.is_some() && since_checkpoint >= self.checkpoint_every {
                since_checkpoint = 0;
                self.save_checkpoint(&queue, &visited);
            }
        }

        // Clean finish: only drop the checkpoint when the crawl genuinely
        // completed (queue drained, no error page we can't recover). A break on
        // max_pages/timeout/errors must KEEP the checkpoint so a resume continues.
        let queue_drained = queue.is_empty() && results.iter().all(|r| r.page.error.is_none());
        if self.crawldir.is_some() && queue_drained {
            self.delete_checkpoint();
        }

        results
    }

    /// BestFirst relevance score: count keyword hits (case-insensitive) in the URL.
    fn score(&self, url: &str) -> i64 {
        if self.keywords.is_empty() {
            return 0;
        }
        let u = url.to_lowercase();
        self.keywords
            .iter()
            .filter(|k| u.contains(&k.to_lowercase()))
            .count() as i64
    }

    /// Enqueue a link. BestFirst keeps the frontier sorted by descending score
    /// (max at front via insertion); BFS/DFS append (front/back pop below).
    /// ponytail: O(n) insertion into a modest frontier, not a BinaryHeap.
    fn push_link(&self, q: &mut std::collections::VecDeque<(String, usize)>, link: String, depth: usize) {
        match self.strategy {
            CrawlStrategy::BestFirst => {
                let s = self.score(&link);
                let pos = q
                    .iter()
                    .position(|(u, _)| self.score(u) < s)
                    .unwrap_or(q.len());
                q.insert(pos, (link, depth));
            }
            _ => q.push_back((link, depth)),
        }
    }

    fn pop(&self, q: &mut std::collections::VecDeque<(String, usize)>) -> Option<(String, usize)> {
        match self.strategy {
            // BestFirst keeps max-score at front
            CrawlStrategy::Bfs | CrawlStrategy::BestFirst => q.pop_front(),
            CrawlStrategy::Dfs => q.pop_back(),
        }
    }
}

// ── Batch operations (multi-tab, enabled by CdpBackend) ──────────────────────
// ponytail: one tab per URL, sequential — the single browser WebSocket serializes
// commands anyway; parallel throughput needs N browser processes (later).

/// Build in-page JS for CSS/XPath-schema extraction (crawl4ai JsonCssExtractionStrategy
/// surface, zero-LLM). Field types: text|attr|html|xpath|regex|nested|nested_list|list.
/// `base_fields` extracts attributes from the container element (e.g. href from <a>).
/// `source` on a field targets the next sibling element (`+ tr`) instead of the container.
/// ponytail: all JS stays in-page via evaluate(), zero deps, zero serialisation overhead.
fn build_field_js(base_fields: &[Value], fields: &[Value]) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('\'', "\\'")
    }
    fn js_name(f: &Value, i: usize) -> String {
        f.get("name")
            .and_then(|v| v.as_str())
            .filter(|n| !n.is_empty())
            .map(|n| serde_json::to_string(n).unwrap_or_default())
            .unwrap_or_else(|| format!("\"field{i}\""))
    }

    let mut field_js = String::new();

    // baseFields: attributes pulled from the container element (e.g. href from <a>)
    for (i, f) in base_fields.iter().enumerate() {
        let n = js_name(f, i);
        let attr = f
            .get("attribute")
            .and_then(|v| v.as_str())
            .unwrap_or("href");
        field_js.push_str(&format!(
            "{n}: (el.getAttribute('{}') ?? null), ",
            attr.replace('\'', "\\'")
        ));
    }

    for (i, f) in fields.iter().enumerate() {
        let n = js_name(f, i + base_fields.len());
        let sel = f.get("selector").and_then(|v| v.as_str()).unwrap_or("");
        let ftype = f.get("type").and_then(|v| v.as_str()).unwrap_or("text");
        let attr = f.get("attr").and_then(|v| v.as_str()).unwrap_or("");
        let re = f.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
        let src = f.get("source").and_then(|v| v.as_str()).unwrap_or("");

        // Target element: el.querySelector(sel), or nextElementSibling when source set
        let tgt = if !src.is_empty() {
            if sel.is_empty() {
                "el.nextElementSibling".to_string()
            } else {
                format!(
                    "(el.nextElementSibling?.querySelector('{}') ?? null)",
                    esc(sel)
                )
            }
        } else if sel.is_empty() {
            "el".to_string()
        } else {
            format!("el.querySelector('{}')", esc(sel))
        };

        let val = match ftype {
            "attr" => format!(
                "({tgt}?.getAttribute('{}') ?? null)",
                attr.replace('\'', "\\'")
            ),
            "html" => format!("({tgt}?.innerHTML ?? null)"),
            "xpath" => format!(
                "(function(){{const r=document.evaluate('{}',el,null,XPathResult.FIRST_ORDERED_NODE_TYPE,null).singleNodeValue;return r?.textContent?.trim()??null;}})()",
                esc(sel)
            ),
            "regex" => {
                let re_escaped = re.replace('\\', "\\\\").replace('\'', "\\'");
                format!(
                    "(function(){{const t={tgt}?.textContent;if(!t)return null;const m=new RegExp('{}','i').exec(t);return m?(m[1]||m[0]).trim():null;}})()",
                    re_escaped
                )
            }
            "nested" => {
                let nf = f.get("fields").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let mut inner = String::new();
                for (j, nff) in nf.iter().enumerate() {
                    let nj = js_name(&nff, j);
                    let ns = nff.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                    let nt = nff.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                    let na = nff.get("attr").and_then(|v| v.as_str()).unwrap_or("");
                    let nv = match nt {
                        "attr" => format!("(c.querySelector('{}')?.getAttribute('{}')??null)", esc(ns), na.replace('\'', "\\'")),
                        "html" => format!("(c.querySelector('{}')?.innerHTML??null)", esc(ns)),
                        _ => format!("(c.querySelector('{}')?.textContent?.trim()??null)", esc(ns)),
                    };
                    inner.push_str(&format!("{nj}: {nv}, "));
                }
                format!("(function(){{const c={tgt}; if(!c)return null; return {{ {inner} }};}})()")
            }
            "nested_list" => {
                let nf = f.get("fields").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let mut inner = String::new();
                for (j, nff) in nf.iter().enumerate() {
                    let nj = js_name(&nff, j);
                    let ns = nff.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                    let nt = nff.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                    let na = nff.get("attr").and_then(|v| v.as_str()).unwrap_or("");
                    let nv = match nt {
                        "attr" => format!("(c.querySelector('{}')?.getAttribute('{}')??null)", esc(ns), na.replace('\'', "\\'")),
                        "html" => format!("(c.querySelector('{}')?.innerHTML??null)", esc(ns)),
                        _ => format!("(c.querySelector('{}')?.textContent?.trim()??null)", esc(ns)),
                    };
                    inner.push_str(&format!("{nj}: {nv}, "));
                }
                format!(
                    "Array.from(el.querySelectorAll('{}')).map(c=>({{ {} }}))",
                    esc(sel), inner
                )
            }
            "list" => format!(
                "Array.from(el.querySelectorAll('{}')).map(c=>c.textContent?.trim()??null)",
                esc(sel)
            ),
            _ => format!("({tgt}?.textContent?.trim() ?? null)"),
        };
        field_js.push_str(&format!("{n}: {val}, "));
    }
    field_js
}

/// Exact CSS extraction: `JSON.stringify(Array.from(document.querySelectorAll(base)).map(extract))`.
pub fn build_extract_js(base_selector: &str, base_fields: &[Value], fields: &[Value]) -> String {
    let field_js = build_field_js(base_fields, fields);
    let base = base_selector.replace('\'', "\\'");
    format!(
        "JSON.stringify(Array.from(document.querySelectorAll('{base}')).map(el => ({{ {field_js} }})))"
    )
}

/// Adaptive extraction (Scrapling-style `adaptive=True`): try the exact base selector, and if it
/// matches nothing (site redesigned/class renamed), relocate to elements that still contain >= 2 of
/// the field selectors, keeping only the deepest (row-level) candidates.
/// ponytail: zero-LLM structural re-anchoring in-page; opt-in via `adaptive: true` on extract_json.
pub fn build_adaptive_extract_js(base_selector: &str, base_fields: &[Value], fields: &[Value]) -> String {
    let field_js = build_field_js(base_fields, fields);
    let base = base_selector.replace('\'', "\\'");
    let sels: Vec<String> = fields
        .iter()
        .filter_map(|f| f.get("selector").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.replace('\'', "\\'"))
        .collect();
    let sel_js = sels.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ");
    format!(
        r#"(() => {{
  const FIELD_SELS = [{sel_js}];
  const extract = (el) => ({{ {field_js} }});
  const exact = Array.from(document.querySelectorAll('{base}')).map(extract);
  if (exact.length) return JSON.stringify(exact);
  if (!FIELD_SELS.length) return JSON.stringify([]);
  // Relocate: find elements that contain >= 2 field selectors, keep the deepest ones.
  const cands = [];
  for (const el of document.querySelectorAll('div, li, article, section, tr, td')) {{
    let hits = 0;
    for (const s of FIELD_SELS) {{ if (el.querySelector(s) && ++hits >= 2) break; }}
    if (hits >= 2) cands.push(el);
  }}
  const minimal = [];
  outer: for (const c of cands) {{
    for (const d of cands) {{ if (d !== c && c.contains(d)) continue outer; }}
    minimal.push(c);
  }}
  return JSON.stringify(minimal.map(extract));
}})()"#
    )
}

/// Built-in regex patterns (crawl4ai RegexExtractionStrategy, 22 patterns, zero-LLM).
/// ponytail: one-liner per pattern; add more on demand.
pub fn regex_patterns() -> Vec<(&'static str, &'static str)> {
    vec![
        ("email", r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
        ("url", r#"https?://[^\s"'<>]+"#),
        ("phone", r"(?:\+?\d{1,3}[-. ]?)?\(?\d{3}\)?[-. ]?\d{3}[-. ]?\d{4}"),
        ("price", r"(?:[$£€]\s?\d{1,3}(?:,\d{3})*(?:\.\d{2})?|\d+(?:\.\d{2})?\s?(?:USD|EUR|GBP))"),
        ("date", r"\b\d{4}[-/]\d{1,2}[-/]\d{1,2}\b|\b\d{1,2}[-/]\d{1,2}[-/]\d{2,4}\b"),
        ("time", r"\b\d{1,2}:\d{2}(?::\d{2})?\s?(?:AM|PM)?\b"),
        ("ip", r"\b(?:\d{1,3}\.){3}\d{1,3}\b"),
        ("uuid", r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b"),
        ("currency", r"(?:[$£€]\s?\d{1,3}(?:,\d{3})*(?:\.\d{2})?|\d+(?:\.\d{2})?\s?(?:USD|EUR|GBP))"),
        ("percentage", r"\b\d+(?:\.\d+)?\s?%\b"),
        ("number", r"\b\d+(?:\.\d+)?\b"),
        ("date_iso", r"\b\d{4}-\d{2}-\d{2}\b"),
        ("date_us", r"\b\d{1,2}/\d{1,2}/\d{2,4}\b"),
        ("time24h", r"\b[012]\d:[0-5]\d(?::[0-5]\d)?\b"),
        ("ipv6", r"\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b|\b::1\b"),
        ("hex_color", r"#(?:[0-9a-fA-F]{3}){1,2}\b"),
        ("postal_us", r"\b\d{5}(?:-\d{4})?\b"),
        ("postal_uk", r"\b[A-Z]{1,2}\d[A-Z\d]?\s?\d[A-Z]{2}\b"),
        ("credit_card", r"\b(?:\d[ -]?){13,19}\b"),
        ("iban", r"\b[A-Z]{2}\d{2}[A-Z0-9]{4,30}\b"),
        ("mac_addr", r"\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b"),
        ("twitter_handle", r"@[A-Za-z0-9_]{1,15}\b"),
        ("hashtag", r"#[A-Za-z0-9_]{1,280}\b"),
    ]
}

/// Build in-page JS for regex extraction. Custom `[{label, re}]` overrides or adds
/// builtins. Scans `document.documentElement.outerHTML` so hrefs/mailto are caught.
/// ponytail: native RegExp in-page, zero new deps, capped 200 matches/pattern.
pub fn build_regex_js(custom: &[Value]) -> String {
    let mut patterns: Vec<(&str, &str)> = regex_patterns();
    for c in custom {
        let l = c.get("label").and_then(|v| v.as_str());
        let re = c.get("re").and_then(|v| v.as_str());
        if let (Some(l), Some(re)) = (l, re) {
            patterns.retain(|(l0, _)| *l0 != l);
            patterns.push((l, re));
        }
    }
    let arr: Vec<String> = patterns
        .iter()
        .map(|(l, re)| {
            format!(
                "{{\"label\":{},\"re\":{}}}",
                serde_json::to_string(l).unwrap_or_default(),
                serde_json::to_string(re).unwrap_or_default()
            )
        })
        .collect();
    format!(
        "(() => {{ const P=[{}]; const src=document.documentElement.outerHTML; \
         const out={{}}; for (const p of P) {{ const re=new RegExp(p.re,'gi'); \
         const seen=new Set(); let m,arr=[]; while ((m=re.exec(src))!==null) {{ \
         const v=(m[0]||'').trim(); if (v && !seen.has(v)) {{ seen.add(v); arr.push(v); }} \
         if (m[0].length===0) re.lastIndex++; }} out[p.label]=arr.slice(0,200); }} \
         return JSON.stringify(out); }})()",
        arr.join(",")
    )
}

/// Run regex extraction over the current page (zero-LLM, in-page). Returns the
/// JSON object `{label: [matches]}`.
pub async fn regex_extract(
    browser: &impl BrowserBackend,
    custom: &[Value],
) -> anyhow::Result<Value> {
    let js = build_regex_js(custom);
    let v = browser.evaluate(&js).await?;
    Ok(v.as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null))
}

/// One row of a batch result. Fields are reused per op:
/// fetch → title/text; extract → data = parsed JSON array (text = same, as JSON
/// string, for backward compat); screenshot → title = saved file path.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BatchResult {
    pub url: String,
    pub title: String,
    pub text: String,
    /// Parsed payload for extract/interact (JSON array); null for fetch/screenshot.
    /// ponytail: mirrors webrain_extract_json's `data` so the LLM reads one shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    pub error: Option<String>,
    /// Wall-clock ms spent on this URL (tab open → result). Lets the LLM see
    /// which URL was slow, and feeds the batch `stats` block.
    #[serde(skip_serializing_if = "is_zero")]
    pub ms: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

fn batch_err(url: &str, e: anyhow::Error) -> BatchResult {
    BatchResult {
        url: url.to_string(),
        title: String::new(),
        text: String::new(),
        data: None,
        error: Some(e.to_string()),
        ms: 0,
    }
}

/// Run one URL on a given (already-open) tab: navigate its session, run
/// `per_url`, return the BatchResult with wall-clock ms. Shared by the parallel
/// path (each URL's own tab) and the single-target sequential fallback (one
/// tab reused for every URL — navigate_session re-navigates it each time).
async fn run_on_tab<F, Fut>(
    browser: &CdpBackend,
    id: String,
    url: String,
    opts: crate::backends::cdp::NavOpts,
    per_url: F,
) -> BatchResult
where
    F: Fn(CdpBackend, String, String) -> Fut + Clone + Sync + Send + 'static,
    Fut: std::future::Future<Output = anyhow::Result<(String, String, Option<Value>)>> + Send + 'static,
{
    let t0 = std::time::Instant::now();
    let res = async {
        let sid = browser.tab_session(&id).await?;
        browser.navigate_session_opts(&sid, &url, &opts).await?;
        let (title, text, data) = per_url(browser.clone(), sid, url.clone()).await?;
        Ok::<_, anyhow::Error>(BatchResult { url: url.clone(), title, text, data, error: None, ms: 0 })
    }
    .await;
    let mut r = match res {
        Ok(r) => r,
        Err(e) => batch_err(&url, e),
    };
    r.ms = t0.elapsed().as_millis() as u64;
    r
}

/// Shared batch skeleton. Multi-target browsers (obscura/Chrome) load N tabs IN
/// PARALLEL (crawl4ai MemoryAdaptiveDispatcher / arun_many): each URL gets its
/// own tab driven via per-session CDP routing; a tokio semaphore bounds
/// in-flight tabs. lightpanda serve is SINGLE-target by design — its CDP holds
/// one browser context and `Target.createTarget` errors `TargetAlreadyLoaded`
/// once one exists (src/cdp/domains/target.zig) — so batch probes tab
/// capability and falls back to running every URL SEQUENTIALLY on one reused
/// tab (page loads serialize, but the whole crawl still works).
/// ponytail: one WS serializes the command layer; parallel page loads overlap
/// on multi-target engines, single-target just accepts the serialization.
async fn batch_map<F, Fut>(
    browser: &CdpBackend,
    urls: &[String],
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
    per_url: F,
) -> Vec<BatchResult>
where
    F: Fn(CdpBackend, String, String) -> Fut + Clone + Sync + Send + 'static,
    Fut: std::future::Future<Output = anyhow::Result<(String, String, Option<Value>)>> + Send + 'static,
{
    // Capability probe (raw CDP — sees the real single-target behavior even
    // when a target already exists): lightpanda errors TargetAlreadyLoaded on
    // any 2nd createTarget → sequential reuse; obscura/Chrome → parallel.
    let single = match browser.single_target_probe().await {
        Ok(s) => s,
        Err(e) => return vec![batch_err(&urls[0], e)],
    };

    if single {
        // lightpanda: reuse one tab for every URL, strictly sequential. Prefer
        // an already-registered tab (e.g. from a prior navigate), else open one.
        let id = match browser.existing_tab().await {
            Some(id) => id,
            None => match browser.open_tab("about:blank").await {
                Ok(id) => id,
                Err(e) => return vec![batch_err(&urls[0], e)],
            },
        };
        let mut out = Vec::with_capacity(urls.len());
        for url in urls {
            out.push(run_on_tab(browser, id.clone(), url.clone(), opts.clone(), per_url.clone()).await);
        }
        return out;
    }

    // Multi-target: original parallel path.
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut handles = Vec::with_capacity(urls.len());
    for url in urls {
        let b = browser.clone();
        let sem = sem.clone();
        let url = url.clone();
        let opts = opts.clone();
        let per_url = per_url.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok();
            let t0 = std::time::Instant::now();
            let res = async {
                let id = b.open_tab(&url).await?;
                let r = run_on_tab(&b, id.clone(), url.clone(), opts, per_url).await;
                let _ = b.close_tab(&id).await;
                Ok::<_, anyhow::Error>(r)
            }
            .await;
            let mut r = res.unwrap_or_else(|e| batch_err(&url, e));
            r.ms = t0.elapsed().as_millis() as u64;
            r
        }));
    }
    let mut out = Vec::with_capacity(urls.len());
    for h in handles {
        if let Ok(r) = h.await {
            out.push(r);
        }
    }
    out
}

/// Batch fetch: read title + visible text per URL, concurrently.
pub async fn batch_fetch(
    browser: &CdpBackend,
    urls: &[String],
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    batch_map(browser, urls, concurrency, opts, |b, sid, _url| async move {
        let title = b.eval_session(&sid, "document.title").await?.as_str().unwrap_or("").to_string();
        let text = b.eval_session(&sid, "document.body ? document.body.innerText || '' : ''").await?
            .as_str().unwrap_or("").to_string();
        Ok((title, text.chars().take(3000).collect(), None))
    }).await
}

/// Batch extraction: run a CSS/XPath schema over every URL concurrently
/// (one tab per URL, semaphore-bounded). Zero-LLM, in-page JS.
pub async fn batch_extract(
    browser: &CdpBackend,
    urls: &[String],
    base_selector: &str,
    base_fields: &[Value],
    fields: &[Value],
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    let js = build_extract_js(base_selector, base_fields, fields);
    batch_map(browser, urls, concurrency, opts, move |b, sid, _url| {
        let js = js.clone();
        async move {
            let v = b.eval_session(&sid, &js).await?;
            let data = v.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap_or(Value::Null);
            Ok((String::new(), data.to_string(), Some(data)))
        }
    }).await
}

/// Batch interaction: run an arbitrary async JS interaction (click "Load More"
/// loop, infinite-scroll, form fill) in PARALLEL tabs, one per URL, then
/// optionally extract a schema. The game-changer for "N independent interactive
/// sites" — one call replaces N serial agent loops (each tab drives its own
/// session; the interaction JS does the waiting).
///
/// `interaction` is async JS that returns nothing (side effects only). If
/// `base_selector` is non-empty, a schema extract runs after the interaction.
pub async fn batch_interact(
    browser: &CdpBackend,
    urls: &[String],
    interaction: &str,
    base_selector: &str,
    base_fields: &[Value],
    fields: &[Value],
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    let interaction = interaction.to_string();
    let extract_js = if base_selector.is_empty() {
        String::new()
    } else {
        build_extract_js(base_selector, base_fields, fields)
    };
    batch_map(browser, urls, concurrency, opts, move |b, sid, _url| {
        let interaction = interaction.clone();
        let extract_js = extract_js.clone();
        async move {
            // Run the interaction; it does its own waits (load-more clicks etc).
            b.eval_session(&sid, &interaction).await?;
            let raw = if extract_js.is_empty() {
                b.eval_session(&sid, "document.body ? document.body.innerText || '' : ''").await?
            } else {
                b.eval_session(&sid, &extract_js).await?
            };
            let data = raw.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok());
            let text = data.clone().unwrap_or(raw);
            Ok((String::new(), text.to_string(), data))
        }
    }).await
}

/// Batch screenshots: save a full-page PNG per URL into `dir`, concurrently
/// (one tab per URL, semaphore-bounded). Honors `NavOpts` (network_idle,
/// disable_resources, wait_selector) like the other batch ops.
pub async fn batch_screenshot(
    browser: &CdpBackend,
    urls: &[String],
    dir: &str,
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    std::fs::create_dir_all(dir).ok();
    let dir = dir.to_string();
    batch_map(browser, urls, concurrency, opts, move |b, sid, url| {
        let dir = dir.clone();
        async move {
            let png = b.screenshot_session(&sid, true).await?;
            // ponytail: short URL hash keeps filenames unique across URLs that
            // share a last path segment (page_x/page_y both ending '/page').
            let mut hasher = sha2::Sha256::new();
            use sha2::Digest;
            hasher.update(url.as_bytes());
            let h = format!("{:x}", hasher.finalize());
            let path = format!("{dir}/page_{}.png", &h[..12]);
            std::fs::write(&path, &png)?;
            Ok((path, format!("{} bytes", png.len()), None))
        }
    }).await
}

/// Chrome-identical HTTP layer for the no-browser fast path (obscura's
/// "GREASE-correct client hints" at the HTTP level). sec-ch-ua follows
/// Chromium's structure — real Chrome brand + GREASE brand (v=24), the form
/// Chrome 101+ actually ships.
/// ponytail: TLS-layer JA3/JA4 byte parity needs a BoringSSL fork (obscura/
/// wreq) — rustls can't emit Chrome's extensions/GREASE. This kills the
/// HTTP-header/UA tells WAFs check first. Add BoringSSL only when a WAF
/// starts failing past this layer.
const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";
const SEC_CH_UA: &str = "\"Google Chrome\";v=\"145\", \"Not)A;Brand\";v=\"24\", \"Chromium\";v=\"145\"";
const BROWSER_HEADERS: &[(&str, &str)] = &[
    ("User-Agent", CHROME_UA),
    ("sec-ch-ua", SEC_CH_UA),
    ("sec-ch-ua-mobile", "?0"),
    ("sec-ch-ua-platform", "\"Windows\""),
    ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8"),
    ("Accept-Language", "en-US,en;q=0.9"),
    ("sec-fetch-site", "none"),
    ("sec-fetch-mode", "navigate"),
    ("sec-fetch-dest", "document"),
    ("upgrade-insecure-requests", "1"),
];

/// Shared HTTP agent — ONE connection pool for all no-browser fetches
/// (http_fetch, validate_urls, download_files). Before, every call built a
/// fresh `ureq::Agent`, so each offset probe paid a new TCP+TLS handshake
/// (~0.3-1s each). Agent is cheap to clone (inner Arc → same pool).
/// ponytail: static OnceLock instead of a builder wrapper per call.
static HTTP_AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();

fn browser_agent() -> ureq::Agent {
    HTTP_AGENT
        .get_or_init(|| {
            ureq::Agent::new_with_config(
                ureq::config::Config::builder()
                    .timeout_global(Some(std::time::Duration::from_secs(30)))
                    .build(),
            )
        })
        .clone()
}

/// Attach the Chrome header set to a no-body request (GET/HEAD).
/// Generic over the request body so we never name ureq's (re-)exported body
/// types; `.header()` lives on `impl<Any> RequestBuilder<Any>`.
fn browser_req<B>(mut req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    for (k, v) in BROWSER_HEADERS {
        req = req.header(*k, *v);
    }
    req
}

/// No-browser HTTP fetch (browsemind `http_crawl`): GET a URL, return status +
/// visible-ish text. 10-100x faster than browser navigation, zero memory — but no
/// JS/SPA/auth. ponytail: ureq GET → plain text; cap to keep MCP replies small.
/// Reuses the shared pooled agent (keep-alive). JSON responses are NOT truncated
/// so a single probe can reveal a total; text/HTML is capped at 3000 chars.
/// Returns pagination headers (X-Total-Count/Link/Content-Range) when present
/// so an agent can discover `total` in one call instead of boundary-probing.
pub fn http_fetch(url: &str) -> anyhow::Result<Value> {
    let resp = browser_req(browser_agent().get(url)).call()?;
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // Pagination / total signals, if the server exposes them (capture before
    // consuming the body).
    let mut hdrs = serde_json::Map::new();
    for name in ["x-total-count", "link", "content-range", "x-next-page"] {
        if let Some(val) = resp.headers().get(name).and_then(|v| v.to_str().ok()) {
            hdrs.insert(name.to_string(), serde_json::Value::String(val.to_string()));
        }
    }
    let text = resp.into_body().read_to_string()?;
    let is_json = content_type.contains("json") || text.trim_start().starts_with(['{', '[']);
    let out_text = if is_json { text } else { text.chars().take(3000).collect::<String>() };
    let mut v = json!({
        "url": url,
        "status": status,
        "content_type": content_type,
        "text": out_text,
        "bytes": out_text.len(),
    });
    if !hdrs.is_empty() {
        v["headers"] = serde_json::Value::Object(hdrs);
    }
    Ok(v)
}

/// Discover crawlable URLs from the site's sitemap (spider-rs `crawl_sitemap` /
/// Scrapling `SitemapSpider`). Flow: robots.txt `Sitemap:` → sitemap_index.xml
/// (or the URL given directly) → leaf sitemaps → every `<loc>`. Uses the pooled
/// HTTP agent (no browser), zero new deps — regex `<loc>` parse, sitemap XML is
/// simple enough. ponytail: no XML parser, `<loc>` regex is correct on the
/// sitemap format; if a server serves a nonstandard sitemap, it yields fewer
/// URLs, the agent falls back to crawl.
/// Returns a JSON object: {urls: [...], sources: [fetched sitemap urls], error?}.
pub fn sitemap_urls(start_url: &str) -> anyhow::Result<Value> {
    fn get_text(url: &str) -> anyhow::Result<String> {
        let resp = browser_req(browser_agent().get(url)).call()?;
        let status = resp.status().as_u16();
        let text = resp.into_body().read_to_string()?;
        if !(200..300).contains(&status) {
            return Err(anyhow::anyhow!("GET {url} -> {status}"));
        }
        Ok(text)
    }
    fn locs(xml: &str) -> Vec<String> {
        // sitemap `<loc>` is CDATA-free; capture the URL inside the tags.
        xml.split("<loc>")
            .skip(1)
            .filter_map(|s| s.split("</loc>").next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
    fn is_index(xml: &str) -> bool {
        // sitemap index contains <sitemap> entries; leaf contains <url>.
        xml.contains("<sitemap>") && !xml.contains("<url>")
    }

    let mut sources = Vec::new();
    let mut urls: Vec<String> = Vec::new();

    // 1. If not given a sitemap URL, ask robots.txt for the Sitemap: line.
    let mut frontier: Vec<String> = Vec::new();
    let seed = start_url.trim_end_matches('/').to_string();
    if seed.contains("/sitemap") || seed.ends_with(".xml") {
        frontier.push(seed);
    } else {
        let robots = format!("{seed}/robots.txt");
        if let Ok(text) = get_text(&robots) {
            // ponytail: multi-line Sitemap: entries; case-insensitive key.
            for line in text.lines() {
                if let Some(idx) = line.to_lowercase().find("sitemap:") {
                    let url = line[idx + 8..].trim();
                    if !url.is_empty() {
                        frontier.push(url.to_string());
                    }
                }
            }
            sources.push(robots);
        }
    }

    let mut seen = std::collections::HashSet::new();
    while let Some(f) = frontier.pop() {
        if !seen.insert(f.clone()) {
            continue;
        }
        sources.push(f.clone());
        match get_text(&f) {
            Ok(xml) => {
                if is_index(&xml) {
                    // sitemap index → push child sitemaps onto the frontier.
                    for child in locs(&xml) {
                        if !seen.contains(&child) {
                            frontier.push(child);
                        }
                    }
                } else {
                    urls.extend(locs(&xml));
                }
            }
            Err(e) => {
                // A 404'd sitemap isn't fatal — keep going through the frontier.
                tracing::debug!("sitemap fetch failed {f}: {e}");
            }
        }
    }

    urls.sort();
    urls.dedup();
    let mut v = json!({"urls": urls, "sources": sources, "count": urls.len()});
    if urls.is_empty() {
        v["error"] = json!("no sitemap URLs found — site may not expose sitemap.xml/robots.txt Sitemap");
    }
    Ok(v)
}

/// BM25 relevance filter (browsemind BM25 filter / crawl4ai ContentRelevanceFilter):
/// score a list of text items against a query, keep the top_k. Zero LLM.
/// ponytail: stdlib-only tokenizer (lowercase, split on non-alphanumeric),
/// k1=1.5, b=0.75, ln+1 IDF. Good enough to rank; no stemming/stopwords.
pub fn bm25_filter(items: &[String], query: &str, top_k: usize) -> Vec<Value> {
    if items.is_empty() || query.trim().is_empty() || top_k == 0 {
        return vec![];
    }
    let k1 = 1.5f64;
    let b = 0.75f64;
    let tokenize = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect()
    };
    let docs: Vec<Vec<String>> = items.iter().map(|i| tokenize(i)).collect();
    let n = docs.len() as f64;
    let avg_len = docs.iter().map(|d| d.len()).sum::<usize>() as f64 / n;
    let q_terms = tokenize(query);

    // ponytail: precompute per-term doc frequency once (O(docs·terms)) instead of
    // re-scanning all docs for every (doc, term) inside the score loop (O(docs²·terms)).
    // Fixes the flagged O(n²) hot path — was linear_scan_in_loop=1 on bm25_filter.
    let mut df: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for d in &docs {
        for t in d {
            *df.entry(t.clone()).or_insert(0.0) += 1.0;
        }
    }

    let mut scored: Vec<(f64, usize)> = Vec::with_capacity(docs.len());
    for (i, doc) in docs.iter().enumerate() {
        let doc_len = doc.len() as f64;
        let mut score = 0.0;
        for t in &q_terms {
            let tf = doc.iter().filter(|d| *d == t).count() as f64;
            if tf == 0.0 {
                continue;
            }
            let df = *df.get(t).unwrap_or(&0.0);
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            let denom = tf + k1 * (1.0 - b + b * doc_len / avg_len);
            score += idf * (tf * (k1 + 1.0)) / denom;
        }
        scored.push((score, i));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(top_k)
        .map(|(s, i)| json!({"index": i, "score": s, "text": items[i]}))
        .collect()
}

/// Validate a list of URLs — which are alive vs dead (404/5xx/errors).
/// browsemind `seed(from_links, validate=True)`. ponytail: HEAD first, GET
/// fallback for HEAD-blocking servers, status < 400 = alive, short timeouts.
pub fn validate_urls(urls: &[String]) -> Vec<Value> {
    let agent = browser_agent();
    urls.iter()
        .map(|u| {
            let alive = match browser_req(agent.head(u)).call() {
                Ok(r) => r.status().as_u16() < 400,
                Err(_) => match browser_req(agent.get(u)).call() {
                    Ok(r) => r.status().as_u16() < 400,
                    Err(_) => false,
                },
            };
            json!({"url": u, "alive": alive})
        })
        .collect()
}

/// Download URLs to `dir` over plain HTTP (no browser). For session-bound files
/// use the browser instead (open a tab, navigate, click).
/// ponytail: ureq stream-to-file via into_reader() (unlimited — read_to_vec caps
/// at 10 MiB, which silently failed on antenna's multi-hundred-MB mp4s);
/// cookie plumbing still pending — add when downloads need auth.
pub fn download_files(urls: &[String], dir: &str) -> Vec<BatchResult> {
    std::fs::create_dir_all(dir).ok();
    let agent = browser_agent();
    let mut out = Vec::with_capacity(urls.len());
    for (i, url) in urls.iter().enumerate() {
        let res = (|| -> anyhow::Result<BatchResult> {
            let resp = browser_req(agent.get(url)).call()?;
            let name = url
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("download");
            let path = format!("{dir}/{i}_{name}");
            let mut f = std::fs::File::create(&path)?;
            let mut reader = resp.into_body().into_reader();
            let n = std::io::copy(&mut reader, &mut f)?;
            Ok(BatchResult {
                url: url.clone(),
                title: path,
                text: format!("{n} bytes"),
                data: None,
                error: None,
                ms: 0,
            })
        })();
        out.push(match res {
            Ok(r) => r,
            Err(e) => batch_err(url, e),
        });
    }
    out
}

// ── Crawl cache ──────────────────────────────────────────────────────────────
// ponytail: SHA-256(url) → {PageState, timestamp} on disk. Same URL re-crawled
// costs the same prompt tokens — a direct token-cost win. Keyed on full URL
// string (query params matter). Args: bypass | enabled | disabled.

pub fn cache_key(url: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(url.as_bytes()))
}

pub fn cache_read(cache_dir: &str, url: &str) -> Option<Value> {
    let path = format!("{cache_dir}/{}.json", cache_key(url));
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn cache_write(cache_dir: &str, url: &str, state: &Value) {
    let _ = std::fs::create_dir_all(cache_dir);
    let path = format!("{cache_dir}/{}.json", cache_key(url));
    let payload = json!({"url": url, "state": state, "ts": chrono_now()});
    let _ = std::fs::write(&path, serde_json::to_string(&payload).unwrap_or_default());
}

fn chrono_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Fit-text cleaning ────────────────────────────────────────────────────────
// ponytail: in-page JS that strips nav/footer/script/style/iframe, excludes
// social/ads links, word_count_threshold. Builds a clean text blob for the LLM
// without an HTML→Markdown converter (deliberate: text via innerText, layout
// via vision tiles). Returns JS string to pass to evaluate().

pub fn build_clean_js(word_threshold: usize, exclude_social: bool) -> String {
    let social = if exclude_social {
        r#"[href*="facebook.com"],[href*="twitter.com"],[href*="instagram.com"],[href*="linkedin.com"],[href*="youtube.com"]"#
    } else {
        ""
    };
    format!(
        r#"(()=>{{const w=document.createElement('div');w.innerHTML=document.body.innerHTML;for(const s of w.querySelectorAll('script,style,noscript,iframe,nav,footer,header,aside,{social}'))s.remove();const t=w.textContent||'';return t.split(/\s+/).filter(w=>w.length>={word_threshold}).join(' ').slice(0,8192);}})()"#
    )
}

// ── PDF extraction ──────────────────────────────────────────────────────────
// Engine: Firecrawl `pdf-inspector` (pure Rust on lopdf) — proper ToUnicode
// CMap decoding, Markdown output, table detection, layout classification,
// AND embedded image extraction (DCTDecode/FlateDecode → PNG via lopdf+image crate).
// Text + images work in the default build; the optional `pdfium` feature adds
// vision rendering (`pdf_render`) + JPEG2000/CCITT image fallback via Google Pdfium.

/// Extract PDF → Markdown + embedded images (Firecrawl pdf-inspector + lopdf).
/// Pure Rust, zero system deps. Returns full-document markdown (headings/lists/
/// tables/bold-italic), per-page text with OCR-needs-flag, layout classification,
/// and embedded images as base64 PNGs (DCTDecode JPEG + FlateDecode raw).
pub fn pdf_extract(path: &str) -> anyhow::Result<Value> {
    let result = pdf_inspector::process_pdf(path)
        .map_err(|e| anyhow::anyhow!("pdf extract failed: {e}"))?;

    let pages_res = pdf_inspector::extract_pages_markdown(path, None)
        .map_err(|e| anyhow::anyhow!("pdf pages failed: {e}"))?;
    let mut texts = serde_json::Map::new();
    for p in &pages_res.pages {
        texts.insert(p.page.to_string(), json!({"text": p.markdown, "needs_ocr": p.needs_ocr}));
    }

    // ponytail: extract images in the same pass. Errors are non-fatal — we
    // still return text+layout even if image extraction fails on exotic formats.
    let images = extract_images_lopdf(path, None).unwrap_or_default();

    Ok(json!({
        "path": path,
        "pages": result.page_count,
        "pdf_type": format!("{:?}", result.pdf_type),
        "confidence": result.confidence,
        "has_encoding_issues": result.has_encoding_issues,
        "layout": {
            "is_complex": result.layout.is_complex,
            "pages_with_tables": result.layout.pages_with_tables,
            "pages_with_columns": result.layout.pages_with_columns,
        },
        "markdown": result.markdown,
        "texts": texts,
        "images": images,
    }))
}

/// Render PDF pages as base64 PNGs — the PixelRAG alternative to text extraction.
/// If `tile_size` is set (e.g. 800), each page is split into square tiles
/// for more efficient vision-model processing. Without tiles, one full image
/// per page. Requires `--features pdfium`.
#[cfg(feature = "pdfium")]
pub fn pdf_render(path: &str, pages: Option<&[u32]>, dpi: Option<f32>, tile_size: Option<u32>) -> anyhow::Result<Value> {
    use pdfium_render::prelude::*;
    use image::GenericImageView;
    let pdfium = Pdfium::default();
    let doc = pdfium.load_pdf_from_file(path, None)?;
    let scale = (dpi.unwrap_or(150.0) / 72.0) as f32;
    let config = PdfRenderConfig::new().scale_page_by_factor(scale);
    let all_pages = doc.pages();
    let target: Vec<u32> = match pages {
        Some(p) => p.to_vec(),
        None => (1..=all_pages.len() as u32).collect(),
    };
    let mut imgs = Vec::new();
    for &pn in &target {
        if let Ok(page) = all_pages.get(pn as i32 - 1) {
            let bmp = page.render_with_config(&config)?;
            let img = bmp.as_image()?.into_rgba8();
            match tile_size {
                Some(ts) => {
                    let (w, h) = img.dimensions();
                    for row in (0..h).step_by(ts as usize) {
                        for col in (0..w).step_by(ts as usize) {
                            let rw = ts.min(w - col);
                            let rh = ts.min(h - row);
                            let tile = image::imageops::crop_imm(&img, col, row, rw, rh).to_image();
                            let mut buf = Vec::new();
                            tile.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
                            use base64::Engine;
                            imgs.push(json!({"page": pn, "x": col, "y": row, "w": rw, "h": rh, "image_b64": base64::engine::general_purpose::STANDARD.encode(&buf)}));
                        }
                    }
                }
                None => {
                    let mut buf = Vec::new();
                    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
                    use base64::Engine;
                    imgs.push(json!({"page": pn, "image_b64": base64::engine::general_purpose::STANDARD.encode(&buf)}));
                }
            }
        }
    }
    Ok(json!({"path": path, "dpi": dpi.unwrap_or(150.0), "tile_size": tile_size, "pages": imgs}))
}

/// Extract embedded images from PDF as base64 PNGs — zero system deps.
/// Uses lopdf to find image XObjects, then `image` crate decodes DCTDecode
/// (JPEG) and FlateDecode (zlib raw pixels) → re-encodes as PNG.
/// Skips JPEG2000/CCITT/JBIG2 — use `webrain_pdf_render` (pdfium) for those.
/// Returns `[{page, index, width, height, image_b64}]`.
pub fn pdf_images(path: &str, pages: Option<&[u32]>) -> anyhow::Result<Value> {
    let imgs = extract_images_lopdf(path, pages)?;
    Ok(json!({"path": path, "count": imgs.len(), "images": imgs}))
}

/// Core image extraction — shared by `pdf_extract` and `pdf_images`.
fn extract_images_lopdf(path: &str, pages: Option<&[u32]>) -> anyhow::Result<Vec<Value>> {
    use flate2::read::ZlibDecoder;
    use lopdf::Document;
    use std::io::Read;

    let doc = Document::load(path)?;
    let all_pages = doc.get_pages();
    let target: Vec<u32> = match pages {
        Some(p) => p.to_vec(),
        None => (1u32..=all_pages.len() as u32).collect(),
    };
    let mut out = Vec::new();
    for &pn in &target {
        let (page_id, _) = match all_pages.get(&pn) {
            Some(p) => *p,
            None => continue,
        };
        // lopdf 0.41: get_pages values are u32 (object number), get_dictionary takes (u32, u16).
        let page_dict = match doc.get_dictionary((page_id, 0)) {
            Ok(d) => d,
            Err(_) => continue,
        };
        // ponytail: direct Resources lookup — skip pages without /Resources
        // (inherited resources from parent nodes are rare in practice).
        let resources = page_dict.get(b"Resources").ok().and_then(|r| r.as_dict().ok());
        if let Some(res) = resources {
            if let Some(xobj) = res.get(b"XObject").ok().and_then(|x| x.as_dict().ok()) {
                let mut idx = 0u32;
                for (_name, obj_val) in xobj.iter() {
                    let obj_id = match obj_val.as_reference().ok() {
                        Some(id) => id,
                        None => continue,
                    };
                    let (content, dict_owned) = match doc.get_object(obj_id) {
                        Ok(lopdf::Object::Stream(s)) => (s.content.clone(), s.dict.clone()),
                        _ => continue,
                    };
                    if dict_owned.get(b"Subtype").ok().and_then(|v| v.as_name().ok()).map(|n| n != b"Image").unwrap_or(true) {
                        continue;
                    }
                    let w = dict_owned.get(b"Width").ok().and_then(|v| v.as_i64().ok()).unwrap_or(0) as u32;
                    let h = dict_owned.get(b"Height").ok().and_then(|v| v.as_i64().ok()).unwrap_or(0) as u32;
                    if w == 0 || h == 0 { continue; }
                    let filter = dict_owned.get(b"Filter").ok().and_then(|v| v.as_name().ok());
                    let data = &content;

                    let img: Option<image::DynamicImage> = match filter {
                        Some(b"DCTDecode") => image::load_from_memory(data).ok(),
                        Some(b"FlateDecode") => {
                            let mut dec = Vec::new();
                            if ZlibDecoder::new(&data[..]).read_to_end(&mut dec).is_err() { continue; }
                            let bpp = dict_owned.get(b"BitsPerComponent").ok().and_then(|v| v.as_i64().ok()).unwrap_or(8) as u8;
                            if bpp != 8 { continue; }
                            let cs = dict_owned.get(b"ColorSpace").ok().and_then(|v| v.as_name().ok());
                            match cs {
                                Some(b"DeviceRGB") => {
                                    image::RgbImage::from_raw(w, h, dec).map(image::DynamicImage::ImageRgb8)
                                }
                                _ => {
                                    image::GrayImage::from_raw(w, h, dec).map(image::DynamicImage::ImageLuma8)
                                }
                            }
                        }
                        _ => None,
                    };

                    if let Some(img) = img {
                        let rgba = img.to_rgba8();
                        let mut buf = Vec::new();
                        rgba.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
                        use base64::Engine;
                        out.push(json!({"page": pn, "index": idx, "width": w, "height": h,
                            "image_b64": base64::engine::general_purpose::STANDARD.encode(&buf)}));
                        idx += 1;
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Batch PDF extraction: extract files CONCURRENTLY. PDF parsing is CPU-bound
/// and single-threaded per file, so real threads (stdlib, no new dep) scale
/// across cores — capped at `available_parallelism()` for massive batches.
/// Returns one JSON object per file, input order preserved.
pub fn pdf_extract_batch(paths: &[String]) -> Vec<Value> {
    let n = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(4);
    let workers = n.min(paths.len()).max(1);
    // ponytail: fixed worker pool, one thread per chunk; per-file threads would
    // thrash on huge batches. Add a concurrency knob only if measured needs it.
    std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(workers);
        for chunk in paths.chunks((paths.len() + workers - 1) / workers) {
            let chunk: Vec<String> = chunk.to_vec();
            handles.push(s.spawn(move || {
                chunk.iter()
                    .map(|p| pdf_extract(p).unwrap_or_else(|e| json!({"path": p, "error": e.to_string()})))
                    .collect::<Vec<Value>>()
            }));
        }
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}
