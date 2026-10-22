// webrain-core/src/httpcache.rs
// Revalidation-only HTTP cache — the ETag half of docs/FEATURE_GAPS.md #8.
//
// Stores the validator (ETag / Last-Modified) plus the body per URL, then sends
// `If-None-Match` / `If-Modified-Since` on the next request. A 304 means the
// SERVER vouches the stored copy is current, so this never serves unvalidated
// stale content — the semantic risk a naive TTL cache carries. A 200 means the
// fresh body wins and replaces the entry.
//
// Keyed by (proxy, url): a different exit IP can be served a different page
// (that is the whole point of rotating), so sharing one entry across proxies
// would hand a caller another region's results.
//
// Two layers, because they answer different questions:
//   * memory — validators AND bodies, this process only.
//   * disk   — validators + a body hash only, survives the process.
// A cold process with no memory entry still finds the disk validator and issues
// a conditional GET. That is what makes SCHEDULED RECURRING MONITORING cheap:
// without it, every check after a restart re-downloads the whole page instead
// of being answered with a 304.
//
// Bodies are deliberately NOT written to disk. The disk may belong to a shared
// worker (one tenant's authenticated page on a volume another tenant can read),
// and a body is orders of magnitude larger than what a monitor needs. What a
// monitor needs is exactly the validator pair, a body hash (so a change stays
// detectable when the origin sends no validator at all) and the check time — so
// that is all the disk layer holds.
//
// Kill switch: `WEBRAIN_HTTP_CACHE=0|off|false|no` disables it entirely —
// `store` is the single gate and it covers both layers, so a process told not
// to cache neither writes the file nor reads one left by an earlier process.
// Location override: `WEBRAIN_HTTP_CACHE_DIR`.
//
// ponytail: the persistent store this module used to defer is now here —
// recurring monitoring is the "a crawl must survive a process restart" case, so
// the condition is met. It is one whole-file rewrite of METADATA (tmp + rename,
// the same idiom as the spider checkpoint), never bodies, and a disk age
// refreshed at most once a minute because a 304 must not pay for it. A
// log-structured or per-entry-file store earns its complexity only when a
// profile says this rewrite matters.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Entries whose validator has not been refreshed in this long are dropped
/// opportunistically on insert. Never causes a read miss on its own — only
/// insertion prunes, so a validated entry always keeps its body.
const TTL: Duration = Duration::from_secs(900);

/// Bounded so a long crawl cannot grow memory without limit.
const MAX_ENTRIES: usize = 256;

/// Disk entries outlive memory entries by two orders of magnitude on purpose.
/// The memory TTL only has to cover one crawl; the disk layer exists to bridge
/// PROCESS RESTARTS, so it has to outlive the monitor's scheduling period — a
/// daily or weekly check must still find its validator, or every run pays a
/// full download and the layer is pointless. Past this, the URL is one we
/// stopped watching.
const DISK_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Count bound for the disk map: 4x MAX_ENTRIES, because after a restart the
/// disk layer is the only layer, and a monitor re-checks everything it once
/// fetched rather than just what is hot right now. At ~250 bytes of metadata
/// per entry this caps the file near 256 KiB — small enough that rewriting it
/// whole is cheap.
const DISK_MAX_ENTRIES: usize = 1024;

/// Refuse to parse a file larger than this. A file this big is not one we
/// wrote (hand-edited, truncated by a full disk, or left by a future layout),
/// and reading it would cost memory to learn nothing. Ignored, not deleted —
/// the next `store` rewrites it.
const DISK_MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// `touch` (a 304) refreshes a disk entry's age at most once per window. A 304
/// is the cheap path this whole module exists to produce, so it must not pay a
/// full-file rewrite; a minute of staleness is irrelevant against a 7-day TTL,
/// and it still means a hot URL cannot age out from under its own validator.
const TOUCH_PERSIST_WINDOW: u64 = 60;

/// Bumped when the on-disk shape changes. A file we do not understand reads as
/// absent, so a downgrade can never misinterpret a newer layout.
const DISK_VERSION: u32 = 1;

const DISK_FILE: &str = "httpcache.json";

struct Entry {
    etag: Option<String>,
    last_modified: Option<String>,
    body: String,
    stored_at: Instant,
}

#[derive(Default)]
struct Cache {
    map: HashMap<String, Entry>,
}

impl Cache {
    fn prune(&mut self, now: Instant) {
        if self.map.len() < MAX_ENTRIES {
            return;
        }
        self.map
            .retain(|_, e| now.duration_since(e.stored_at) < TTL);
        if self.map.len() >= MAX_ENTRIES {
            // Still full of live entries: evict the least recently validated.
            if let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, e)| e.stored_at)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&oldest);
            }
        }
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

/// Everything a restart needs and nothing else. The URL itself is not here: the
/// key is hashed (see `key_hash`) because a query string can carry a signed
/// token or an API key, and this file is written to a disk we do not control.
#[derive(Serialize, Deserialize)]
struct Meta {
    etag: Option<String>,
    last_modified: Option<String>,
    /// sha256 hex of the body, so a change is still detectable when the origin
    /// sends no validator at all.
    sha256: String,
    /// Unix seconds. `Entry::stored_at` is an `Instant`, which is meaningless
    /// across a restart — the one reason this field is not shared with it.
    checked_at: u64,
}

/// Read shape.
#[derive(Deserialize)]
struct DiskFile {
    version: u32,
    entries: HashMap<String, Meta>,
}

/// Write shape — borrows the map so persisting does not clone every entry.
#[derive(Serialize)]
struct DiskFileRef<'a> {
    version: u32,
    entries: &'a HashMap<String, Meta>,
}

/// Read a store file. Every failure mode collapses to `None`, which the caller
/// treats as an empty store: absent, unreadable, oversized, wrong version,
/// malformed JSON, or truncated by a crash mid-write.
fn read_file(path: &Path) -> Option<HashMap<String, Meta>> {
    if std::fs::metadata(path).ok()?.len() > DISK_MAX_FILE_BYTES {
        return None;
    }
    let parsed: DiskFile = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    if parsed.version != DISK_VERSION {
        return None;
    }
    Some(parsed.entries)
}

/// The persistent layer: metadata for URLs this machine has fetched, so the NEXT
/// process asks a conditional question instead of downloading the page again.
///
/// `path: None` means inert (kill switch on, or no directory resolvable), and
/// the invariant is that `map` is non-empty only when `path` is `Some` — an
/// inert store always misses and never writes.
struct DiskStore {
    path: Option<PathBuf>,
    map: HashMap<String, Meta>,
}

impl DiskStore {
    fn load(path: Option<PathBuf>) -> Self {
        let map = path.as_deref().and_then(read_file).unwrap_or_default();
        let mut store = Self { path, map };
        // Expire on load, not only on write: a file untouched for longer than
        // DISK_TTL is stale the moment it is read.
        store.prune(unix_secs());
        store
    }

    /// Drop entries past DISK_TTL, then trim to the newest DISK_MAX_ENTRIES.
    fn prune(&mut self, now: u64) {
        self.map
            .retain(|_, m| now.saturating_sub(m.checked_at) < DISK_TTL.as_secs());
        if self.map.len() <= DISK_MAX_ENTRIES {
            return;
        }
        // Oldest first, then everything past the cap. Keys are collected by
        // value because the map cannot be borrowed while it is being mutated.
        let mut by_age: Vec<(u64, String)> = self
            .map
            .iter()
            .map(|(k, m)| (m.checked_at, k.clone()))
            .collect();
        by_age.sort_unstable();
        let excess = self.map.len() - DISK_MAX_ENTRIES;
        for (_, k) in by_age.into_iter().take(excess) {
            self.map.remove(&k);
        }
    }

    /// Record a 200's metadata. An inert store remembers nothing, so the kill
    /// switch holds here as well as at `store`'s gate.
    fn record(&mut self, k: &str, etag: Option<String>, last_modified: Option<String>, body: &str) {
        if self.path.is_none() {
            return;
        }
        let now = unix_secs();
        self.map.insert(
            key_hash(k),
            Meta {
                etag,
                last_modified,
                sha256: sha256_hex(body.as_bytes()),
                checked_at: now,
            },
        );
        self.prune(now);
        self.persist();
    }

    fn lookup(&self, k: &str) -> Option<(Option<String>, Option<String>)> {
        self.map
            .get(&key_hash(k))
            .map(|m| (m.etag.clone(), m.last_modified.clone()))
    }

    fn fingerprint(&self, k: &str) -> Option<&str> {
        self.map.get(&key_hash(k)).map(|m| m.sha256.as_str())
    }

    /// Write the whole map atomically: temp file, then rename, so a crash leaves
    /// the previous file intact instead of a half-written one (the same idiom as
    /// the spider checkpoint). Every failure is swallowed — an unwritable cache
    /// costs a full download next run and must never cost a failed fetch.
    fn persist(&self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let Ok(body) = serde_json::to_string(&DiskFileRef {
            version: DISK_VERSION,
            entries: &self.map,
        }) else {
            return;
        };
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn disk() -> &'static Mutex<DiskStore> {
    static DISK: OnceLock<Mutex<DiskStore>> = OnceLock::new();
    DISK.get_or_init(|| {
        // Path resolved once, at first use. The kill switch is applied here and
        // not only at write time, so a process told not to cache does not send a
        // conditional GET built from a previous process's file; the gate in
        // `store` covers the write half.
        let path = if disabled() { None } else { disk_path() };
        Mutex::new(DiskStore::load(path))
    })
}

/// Where the metadata file lives. `WEBRAIN_HTTP_CACHE_DIR` wins; otherwise the
/// platform's CACHE directory (same ladder as `install::browsers_dir`), so
/// per-tenant state does not land in a shared, world-writable temp dir. Temp is
/// the last resort only: a wiped file costs a cache miss, nothing more.
fn disk_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WEBRAIN_HTTP_CACHE_DIR") {
        return Some(PathBuf::from(p).join(DISK_FILE));
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return Some(PathBuf::from(local).join("webrain").join(DISK_FILE));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return Some(
                PathBuf::from(home)
                    .join("Library/Caches/webrain")
                    .join(DISK_FILE),
            );
        }
    }
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        return Some(PathBuf::from(x).join("webrain").join(DISK_FILE));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Some(PathBuf::from(home).join(".cache/webrain").join(DISK_FILE));
    }
    Some(std::env::temp_dir().join("webrain").join(DISK_FILE))
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    // sha2 0.11's output no longer implements LowerHex; format manually.
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Hash of `key(proxy, url)`. Hashing instead of storing the key keeps the
/// (proxy, url) distinction exact — distinct keys hash apart, so one URL through
/// two exits still keeps its own validator — while keeping a signed URL with its
/// token out of a file on disk.
fn key_hash(k: &str) -> String {
    sha256_hex(k.as_bytes())
}

/// `WEBRAIN_HTTP_CACHE=0|off|false|no` turns the cache off.
pub fn disabled() -> bool {
    is_off(std::env::var("WEBRAIN_HTTP_CACHE").ok().as_deref())
}

/// Split out from `disabled()` so the accepted strings are testable without
/// mutating the process environment (which cargo shares across test threads).
fn is_off(v: Option<&str>) -> bool {
    match v {
        Some(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        None => false,
    }
}

/// Cache key. The proxy is part of it because the same URL through two exits
/// is not the same resource for our purposes.
pub fn key(proxy: Option<&str>, url: &str) -> String {
    format!("{}\u{1}{}", proxy.unwrap_or(""), url)
}

/// `Some((etag, last_modified))` when an entry exists. Both may be `None` —
/// then nothing is sendable and the caller just requests unconditionally.
///
/// Memory first; a memory miss falls through to the disk layer, which is the
/// whole point: a freshly started process still sends `If-None-Match` instead of
/// re-downloading everything it monitored yesterday. The two locks are taken one
/// at a time, never nested, so a concurrent `store` cannot deadlock this — and
/// losing that race costs one unconditional GET, nothing more.
pub fn validators(k: &str) -> Option<(Option<String>, Option<String>)> {
    {
        let g = cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = g.map.get(k) {
            return Some((e.etag.clone(), e.last_modified.clone()));
        }
    }
    let d = disk().lock().unwrap_or_else(|e| e.into_inner());
    d.lookup(k)
}

/// Stored body for a revalidated (304) response.
///
/// Bodies are memory-only (see the module header), so a process that just
/// started can be handed a 304 it has no body for. The caller already handles
/// that case — it re-requests unconditionally when the stored body is gone.
pub fn body(k: &str) -> Option<String> {
    let g = cache().lock().unwrap_or_else(|e| e.into_inner());
    g.map.get(k).map(|e| e.body.clone())
}

/// sha256 hex of the body the origin last served for `k`, from the persistent
/// layer.
///
/// This is the change signal for an origin that sends neither ETag nor
/// Last-Modified: there is no validator pair to revalidate with, so the hash is
/// the only way a monitor can tell that the page changed.
pub fn fingerprint(k: &str) -> Option<String> {
    let d = disk().lock().unwrap_or_else(|e| e.into_inner());
    d.fingerprint(k).map(str::to_string)
}

/// Record a fresh 200 response: memory keeps the body, disk keeps metadata.
pub fn store(k: &str, etag: Option<String>, last_modified: Option<String>, body: &str) {
    if disabled() {
        // One gate for both layers — disk must switch off with memory, or the
        // kill switch only half works. `DiskStore::record` repeats the check for
        // the case where no directory was resolvable.
        return;
    }
    let now = Instant::now();
    {
        let mut g = cache().lock().unwrap_or_else(|e| e.into_inner());
        g.prune(now);
        g.map.insert(
            k.to_string(),
            Entry {
                etag: etag.clone(),
                last_modified: last_modified.clone(),
                body: body.to_string(),
                stored_at: now,
            },
        );
    }
    // Hashing and the file write happen outside the memory lock, so a slow disk
    // cannot stall a reader that only wants a validator.
    let mut d = disk().lock().unwrap_or_else(|e| e.into_inner());
    d.record(k, etag, last_modified, body);
}

/// Refresh an entry's age after the server revalidated it (304), so a busy URL
/// is never pruned out from under its own validators.
pub fn touch(k: &str) {
    let now = Instant::now();
    {
        let mut g = cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = g.map.get_mut(k) {
            e.stored_at = now;
        }
    }
    // The disk age is refreshed too, but at most once per TOUCH_PERSIST_WINDOW:
    // a 304 is the cheap path this module exists to produce, and it must not pay
    // a full-file rewrite to keep alive an entry that is nowhere near its 7-day
    // TTL. The file lags by at most that window.
    let now_secs = unix_secs();
    let mut d = disk().lock().unwrap_or_else(|e| e.into_inner());
    let h = key_hash(k);
    let due = d
        .map
        .get(&h)
        .is_some_and(|m| now_secs.saturating_sub(m.checked_at) >= TOUCH_PERSIST_WINDOW);
    if due {
        if let Some(m) = d.map.get_mut(&h) {
            m.checked_at = now_secs;
        }
        d.persist();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_then_serve_validators_and_body() {
        let k = unique_key("plain");
        assert!(validators(&k).is_none(), "a key never stored must miss");
        store(
            &k,
            Some("\"abc\"".into()),
            Some("Wed, 21 Oct 2026 07:28:00 GMT".into()),
            "<html>cached</html>",
        );
        let (etag, lm) = validators(&k).expect("entry exists");
        assert_eq!(etag.as_deref(), Some("\"abc\""));
        assert_eq!(lm.as_deref(), Some("Wed, 21 Oct 2026 07:28:00 GMT"));
        assert_eq!(body(&k).as_deref(), Some("<html>cached</html>"));
    }

    #[test]
    fn key_separates_same_url_across_proxies() {
        let a = key(Some("http://p1:1"), "https://x.test/s");
        let b = key(Some("http://p2:1"), "https://x.test/s");
        let none = key(None, "https://x.test/s");
        assert_ne!(a, b, "different exits must not share an entry");
        assert_ne!(a, none);
    }

    #[test]
    fn touch_keeps_a_validated_entry_from_ageing_out() {
        let k = "test-key-touch";
        store(k, Some("\"v\"".into()), None, "body");
        touch(k);
        assert_eq!(body(k).as_deref(), Some("body"));
    }

    #[test]
    fn eviction_is_bounded_and_keeps_the_newest() {
        let mut c = Cache::default();
        let now = Instant::now();
        for i in 0..MAX_ENTRIES {
            c.map.insert(
                format!("k{i}"),
                Entry {
                    etag: None,
                    last_modified: None,
                    body: String::new(),
                    stored_at: now,
                },
            );
        }
        for i in 0..MAX_ENTRIES {
            c.prune(now);
            if c.map.len() < MAX_ENTRIES {
                break;
            }
            // All same age: prune must still make room rather than grow.
            c.map.remove(&format!("k{i}"));
        }
        assert!(c.map.len() < MAX_ENTRIES);
    }

    // ----------------------------------------------------------------- disk ---

    /// A private directory per test (`None`-path stores never touch it). Distinct
    /// tags keep the tests independent — cargo runs them in one process.
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("webrain-httpcache-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn tmp_path(tag: &str) -> PathBuf {
        tmp_dir(tag).join(DISK_FILE)
    }

    /// A key no earlier run can have stored. The disk layer persists across runs
    /// by design, so a fixed literal would make "this key is a miss" true only on
    /// the first execution on a machine.
    fn unique_key(tag: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("test-{tag}-{nanos}")
    }

    #[test]
    fn is_off_accepts_the_documented_kill_switch_values() {
        for on in [
            None,
            Some(""),
            Some("1"),
            Some("yes"),
            Some("on"),
            Some("junk"),
        ] {
            assert!(!is_off(on), "{on:?} must leave the cache enabled");
        }
        for off in ["0", "off", "false", "no", " OFF ", "False"] {
            assert!(is_off(Some(off)), "{off:?} must disable the cache");
        }
    }

    #[test]
    fn sha256_hex_matches_the_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn metadata_survives_a_restart() {
        let path = tmp_path("restart");
        {
            // First "process": store, then drop the store like an exit would.
            let mut a = DiskStore::load(Some(path.clone()));
            a.record(
                "k1",
                Some("\"e1\"".into()),
                Some("Wed, 21 Oct 2026 07:28:00 GMT".into()),
                "body-one",
            );
        }
        // Second "process": nothing in memory, and the validator is still there.
        let b = DiskStore::load(Some(path.clone()));
        let (etag, lm) = b.lookup("k1").expect("metadata must survive a restart");
        assert_eq!(etag.as_deref(), Some("\"e1\""));
        assert_eq!(lm.as_deref(), Some("Wed, 21 Oct 2026 07:28:00 GMT"));
        assert_eq!(b.fingerprint("k1"), Some(sha256_hex(b"body-one").as_str()));
        assert!(b.lookup("never-stored").is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new(".")));
    }

    #[test]
    fn persisted_state_does_not_collapse_two_exits_for_one_url() {
        let path = tmp_path("exits");
        let url = "https://x.test/s";
        let p1 = key(Some("http://p1:1"), url);
        let p2 = key(Some("http://p2:1"), url);
        let mut a = DiskStore::load(Some(path.clone()));
        a.record(&p1, Some("\"p1\"".into()), None, "via-exit-1");
        a.record(&p2, Some("\"p2\"".into()), None, "via-exit-2");
        let b = DiskStore::load(Some(path.clone()));
        assert_eq!(
            b.lookup(&p1).and_then(|(e, _)| e).as_deref(),
            Some("\"p1\"")
        );
        assert_eq!(
            b.lookup(&p2).and_then(|(e, _)| e).as_deref(),
            Some("\"p2\"")
        );
        // The no-proxy key is a third resource, not an alias for either.
        assert!(b.lookup(&key(None, url)).is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new(".")));
    }

    /// The end-to-end contract of the feature, through the public API only: a
    /// validator stored by one process is sendable by the next. Memory is wiped
    /// by hand because that is the one thing a restart does that a test cannot
    /// do to a `'static`.
    #[test]
    fn a_cold_process_still_reads_a_validator_a_previous_one_stored() {
        assert!(
            !disabled(),
            "this test needs the cache on; WEBRAIN_HTTP_CACHE is set in the environment"
        );
        let k = unique_key("cold-process");
        store(
            &k,
            Some("\"cold\"".into()),
            Some("Wed, 21 Oct 2026 07:28:00 GMT".into()),
            "body",
        );
        {
            let mut g = cache().lock().unwrap_or_else(|e| e.into_inner());
            g.map.remove(&k);
            assert!(!g.map.contains_key(&k), "memory must be empty");
        }
        let (etag, lm) = validators(&k).expect("the disk layer must answer a memory miss");
        assert_eq!(etag.as_deref(), Some("\"cold\""));
        assert_eq!(lm.as_deref(), Some("Wed, 21 Oct 2026 07:28:00 GMT"));
        assert_eq!(
            fingerprint(&k).as_deref(),
            Some(sha256_hex(b"body").as_str())
        );
    }

    #[test]
    fn an_inert_store_persists_nothing() {
        let path = tmp_path("off");
        // `None` is what `disk()` passes when the kill switch is on, so this is
        // the disabled layer: it must remember nothing and write no file.
        let mut s = DiskStore::load(None);
        s.record("k", Some("\"v\"".into()), None, "body");
        assert!(s.lookup("k").is_none());
        assert!(s.fingerprint("k").is_none());
        assert!(!path.exists(), "the kill switch must not create a file");
    }

    #[test]
    fn the_disk_store_is_bounded_and_keeps_the_newest() {
        let mut s = DiskStore::load(None);
        let now = unix_secs();
        for i in 0..(DISK_MAX_ENTRIES + 64) {
            s.map.insert(
                format!("h{i}"),
                Meta {
                    etag: None,
                    last_modified: None,
                    sha256: String::new(),
                    // Newest first, and all well inside DISK_TTL so only the
                    // count bound can be doing the evicting.
                    checked_at: now - i as u64,
                },
            );
        }
        s.prune(now);
        assert_eq!(s.map.len(), DISK_MAX_ENTRIES);
        assert!(s.map.contains_key("h0"), "the newest must survive");
        assert!(
            !s.map.contains_key(&format!("h{}", DISK_MAX_ENTRIES + 63)),
            "the oldest must be evicted rather than growing the store"
        );
    }

    #[test]
    fn entries_past_the_disk_ttl_are_dropped_without_being_counted() {
        let mut s = DiskStore::load(None);
        let now = unix_secs();
        let fresh = |checked_at| Meta {
            etag: None,
            last_modified: None,
            sha256: String::new(),
            checked_at,
        };
        s.map.insert("fresh".into(), fresh(now));
        s.map
            .insert("stale".into(), fresh(now - DISK_TTL.as_secs() - 1));
        s.prune(now);
        assert!(s.map.contains_key("fresh"));
        assert!(!s.map.contains_key("stale"));
    }

    #[test]
    fn corrupt_or_absent_state_reads_as_a_miss_and_never_panics() {
        let path = tmp_path("corrupt");
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        std::fs::create_dir_all(&dir).expect("temp dir");

        let junk = [
            "",
            "not json at all",
            "{\"version\":1,\"entries\":{\"a\":{\"etag\":\"x\"}}}", // partial entry
            "{\"version\":99,\"entries\":{}}",                      // future layout
            "{\"version\":1,\"entries\":{\"a\":",                   // truncated mid-write
            "{\"entries\":{}}",                                     // no version field
        ];
        for blob in junk {
            std::fs::write(&path, blob).expect("write junk");
            let s = DiskStore::load(Some(path.clone()));
            assert!(
                s.lookup("anything").is_none(),
                "{:?} must read as an empty store",
                &blob[..blob.len().min(24)]
            );
        }

        // A file too large to be one of ours is ignored, not parsed.
        let oversized = "x".repeat((DISK_MAX_FILE_BYTES + 1) as usize);
        std::fs::write(&path, &oversized).expect("write oversized");
        assert!(DiskStore::load(Some(path.clone())).lookup("k").is_none());

        // A directory that does not exist is a miss too, not an error.
        assert!(
            DiskStore::load(Some(dir.join("gone").join(DISK_FILE)))
                .lookup("k")
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recording_creates_a_missing_directory() {
        let path = tmp_dir("mkdir")
            .join("nested")
            .join("deeper")
            .join(DISK_FILE);
        let mut s = DiskStore::load(Some(path.clone()));
        s.record("k", Some("\"v\"".into()), None, "body");
        assert!(path.is_file(), "record must create the directory and write");
        let r = DiskStore::load(Some(path.clone()));
        assert_eq!(r.lookup("k").and_then(|(e, _)| e).as_deref(), Some("\"v\""));
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new(".")));
    }
}
