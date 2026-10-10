// webrain-core/src/proxy.rs
// Proxy rotation pool — the missing half of the per-request `proxy` option
// (docs/FEATURE_GAPS.md 9b).
//
// `WEBRAIN_PROXIES` is a comma- or newline-separated list of proxy URLs.
// Requests round-robin across the live ones; an exit that hits a wall
// (403/429/503, or a transport reset) is marked dead for a cooldown so one
// transient rate-limit cannot permanently poison a good IP.
//
// An explicit per-call `proxy` ALWAYS wins. The pool is only the fallback when
// the caller names none, which keeps single-proxy deployments byte-identical
// to the pre-pool behavior and makes this feature opt-in by env var alone.
//
// Scope: this rotates the NO-BROWSER HTTP path (SERP). The browser path cannot
// rotate per request — a CDP engine's proxy is baked in at launch
// (`launch.rs --proxy-server`), so per-proxy isolation there is a pool of
// browsers handed to `webrain_batch` as `cdp_urls`, which already works.
//
// ponytail: process-global in-memory pool, no persistence; a hosted
// multi-tenant deployment wants per-tenant lanes and durable health state.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Env var holding the rotation list.
pub const ENV: &str = "WEBRAIN_PROXIES";

/// A walled proxy stays out of rotation this long before it is retried.
/// Long enough to outlast a burst rate-limit, short enough that a false
/// positive costs seconds rather than the whole run.
const DEAD_COOLDOWN: Duration = Duration::from_secs(120);

/// Round-robin pool with a cooldown on walled exits.
pub struct ProxyPool {
    proxies: Vec<String>,
    cursor: usize,
    dead: HashMap<String, Instant>,
}

impl ProxyPool {
    /// Split a raw list on commas/newlines, trim, drop empties, dedupe.
    /// Order is preserved — the first entry is the first one used.
    pub fn parse(raw: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for part in raw.split([',', '\n', '\r']) {
            let p = part.trim();
            if p.is_empty() || out.iter().any(|x| x == p) {
                continue;
            }
            out.push(p.to_string());
        }
        out
    }

    /// Build from `WEBRAIN_PROXIES`. `None` when unset/empty, so callers treat
    /// "no pool configured" and "pool disabled" identically.
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var(ENV).ok()?;
        let proxies = Self::parse(&raw);
        if proxies.is_empty() {
            return None;
        }
        Some(Self {
            proxies,
            cursor: 0,
            dead: HashMap::new(),
        })
    }

    /// Next live proxy, advancing the cursor. Named `acquire` rather than
    /// `next` so it cannot be mistaken for `Iterator::next` — this is not a
    /// sequence, it is one exit IP taken for one request.
    ///
    /// When every entry is cooling down the one dead longest is returned rather
    /// than `None`: a pool that answers "no proxy" would strand the caller
    /// mid-crawl, and a possibly-walled exit still beats no request at all.
    pub fn acquire(&mut self) -> Option<String> {
        self.next_at(Instant::now())
    }

    /// Record a wall so this proxy rotates out for `DEAD_COOLDOWN`.
    pub fn mark_dead(&mut self, proxy: &str) {
        self.mark_dead_at(proxy, Instant::now());
    }

    fn next_at(&mut self, now: Instant) -> Option<String> {
        let n = self.proxies.len();
        if n == 0 {
            return None;
        }
        for _ in 0..n {
            let idx = self.cursor % n;
            self.cursor = self.cursor.wrapping_add(1);
            if self.is_live_at(&self.proxies[idx], now) {
                return Some(self.proxies[idx].clone());
            }
        }
        // Every exit is cooling down. Clear so rotation resumes instead of
        // stalling — the caller still gets a request out.
        self.dead.clear();
        let idx = self.cursor % n;
        self.cursor = self.cursor.wrapping_add(1);
        Some(self.proxies[idx].clone())
    }

    fn mark_dead_at(&mut self, proxy: &str, now: Instant) {
        // Only pool members are tracked; an explicit per-call proxy is the
        // caller's business and must not enter (or poison) the rotation.
        if self.proxies.iter().any(|p| p == proxy) {
            self.dead.insert(proxy.to_string(), now);
        }
    }

    fn is_live_at(&self, proxy: &str, now: Instant) -> bool {
        match self.dead.get(proxy) {
            Some(since) => now.duration_since(*since) >= DEAD_COOLDOWN,
            None => true,
        }
    }
}

/// Process-global pool. `None` when `WEBRAIN_PROXIES` is unset — the common case.
pub fn pool() -> Option<&'static Mutex<ProxyPool>> {
    static POOL: OnceLock<Option<Mutex<ProxyPool>>> = OnceLock::new();
    POOL.get_or_init(|| ProxyPool::from_env().map(Mutex::new))
        .as_ref()
}

/// Next live proxy from the global pool. Poison-tolerant: a panic in another
/// thread must not take rotation down.
pub fn next_proxy() -> Option<String> {
    let p = pool()?;
    let mut g = p.lock().unwrap_or_else(|e| e.into_inner());
    g.acquire()
}

/// Mark a global-pool proxy as walled.
pub fn mark_dead(proxy: &str) {
    if let Some(p) = pool() {
        let mut g = p.lock().unwrap_or_else(|e| e.into_inner());
        g.mark_dead(proxy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_of(items: &[&str]) -> ProxyPool {
        ProxyPool {
            proxies: items.iter().map(|s| s.to_string()).collect(),
            cursor: 0,
            dead: HashMap::new(),
        }
    }

    #[test]
    fn parse_trims_dedupes_and_splits_both_separators() {
        let got = ProxyPool::parse(" http://a:1 ,\nsocks5://b:2 , ,http://a:1\n");
        assert_eq!(got, vec!["http://a:1", "socks5://b:2"]);
        assert!(ProxyPool::parse("   ,\n ").is_empty());
    }

    #[test]
    fn rotation_round_robins_then_skips_the_walled_exit() {
        let mut p = pool_of(&["a", "b"]);
        let t0 = Instant::now();
        assert_eq!(p.next_at(t0).as_deref(), Some("a"));
        assert_eq!(p.next_at(t0).as_deref(), Some("b"));
        p.mark_dead_at("a", t0);
        assert_eq!(p.next_at(t0).as_deref(), Some("b"), "skips the walled exit");
        assert_eq!(p.next_at(t0).as_deref(), Some("b"));
    }

    #[test]
    fn walled_proxy_is_live_again_after_the_cooldown() {
        let mut p = pool_of(&["a"]);
        let t0 = Instant::now();
        p.mark_dead_at("a", t0);
        assert!(!p.is_live_at("a", t0));
        assert!(!p.is_live_at("a", t0 + DEAD_COOLDOWN - Duration::from_secs(1)));
        assert!(p.is_live_at("a", t0 + DEAD_COOLDOWN));
    }

    #[test]
    fn all_dead_still_returns_a_proxy() {
        let mut p = pool_of(&["a"]);
        let t0 = Instant::now();
        p.mark_dead_at("a", t0);
        assert_eq!(
            p.next_at(t0).as_deref(),
            Some("a"),
            "never strand the caller"
        );
    }

    #[test]
    fn foreign_proxy_is_not_tracked() {
        let mut p = pool_of(&["a"]);
        let t0 = Instant::now();
        p.mark_dead_at("not-in-pool", t0);
        assert!(
            p.dead.is_empty(),
            "an explicit proxy must not enter rotation"
        );
        assert_eq!(p.next_at(t0).as_deref(), Some("a"));
    }

    #[test]
    fn empty_pool_yields_nothing() {
        let mut p = pool_of(&[]);
        assert_eq!(p.acquire(), None);
    }
}
