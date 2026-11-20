//! Batch memory and throughput soak.
//!
//! Runs the real engine path, `batch_fetch`, against a local static page at each
//! concurrency level and records wall time plus peak RSS. This is the number the
//! memory contract is held to: 10, 100 and 1000 URLs must stay in one footprint.
//!
//! ```text
//! cargo run --release --example soak
//! ```
//!
//! Env:
//!   CDP_URL      CDP endpoint, `http://127.0.0.1:9222` or a `ws://` URL. Required;
//!                without it the soak prints how to start a browser and exits 0.
//!   SOAK_URLS    rows per run (default 200)
//!   SOAK_LEVELS  concurrency levels (default 1,4,8,16,32)
//!   SOAK_MIN_TITLE  reject rows whose title is not this string (catches "ok rows
//!                that loaded nothing"), default "soak"
//!
//! Output is a markdown table; paste it next to the date in the perf notes.
//!
//! Shortcut: one run per level, so this shows peak and trend, not p95. Add
//! repetitions when a level's numbers start deciding a change.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessesToUpdate, System};
use webrain_core::backends::cdp::{CdpBackend, NavOpts};
use webrain_core::engines::batch_fetch;

const PAGE: &str = r#"<!doctype html><html><head><title>soak</title></head><body>
<main>
<div class="product"><a href="/p/1" class="name">Item 1</a><span class="price">19.99</span></div>
<div class="product"><a href="/p/2" class="name">Item 2</a><span class="price">24.50</span></div>
<div class="product"><a href="/p/3" class="name">Item 3</a><span class="price">31.00</span></div>
<p>Local soak fixture. Roughly 4 KB so a row costs what a real small page costs.</p>
<p>Padding: rust browser automation over CDP keeps extraction deterministic.</p>
</main></body></html>"#;

/// Minimal static server. `Connection: close` per request so no keep-alive state
/// hides in the fixture; one thread per connection, capped by the OS.
///
/// `flaky` turns it into the failure-injection row of the benchmark matrix:
/// every 3rd request answers 500 and every 7th answers 429, so the batch has to
/// account for failures instead of assuming every URL loads.
fn serve(flaky: bool) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind soak server");
    let port = listener.local_addr().expect("local addr").port();
    let hits = Arc::new(AtomicU64::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let n = hits.fetch_add(1, Ordering::Relaxed);
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let (status, body): (&str, Vec<u8>) = match (flaky, n % 7, n % 3) {
                    (true, 6, _) => ("429 Too Many Requests", b"slow down".to_vec()),
                    (true, _, 2) => ("500 Internal Server Error", b"boom".to_vec()),
                    _ => ("200 OK", PAGE.as_bytes().to_vec()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            });
        }
    });
    port
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn env_levels(key: &str, default: &[usize]) -> Vec<usize> {
    match std::env::var(key) {
        Ok(v) => v
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .filter(|n| *n > 0)
            .collect(),
        Err(_) => default.to_vec(),
    }
}

fn rss_bytes(sys: &mut System) -> u64 {
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.process(Pid::from_u32(std::process::id()))
        .map(|p| p.memory())
        .unwrap_or(0)
}

/// Summed RSS of every browser process the soak might be driving.
///
/// Process-wide by name, so it includes any browser the operator already has
/// open; the table reports it as a DELTA against a baseline taken before the
/// first level, which is what makes the tabs attributable.
fn browser_rss_bytes(sys: &mut System) -> u64 {
    const NAMES: &[&str] = &[
        "chrome",
        "chromium",
        "msedge",
        "moli",
        "obscura",
        "lightpanda",
    ];
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .filter(|p| {
            let n = p.name().to_string_lossy().to_ascii_lowercase();
            NAMES.iter().any(|k| n.contains(k))
        })
        .map(|p| p.memory())
        .sum()
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Samples this process and the browser RSS on a background thread for one run.
struct Sampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    peak_browser: Arc<AtomicU64>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(0));
        let peak_browser = Arc::new(AtomicU64::new(0));
        let (s, p, pb) = (stop.clone(), peak.clone(), peak_browser.clone());
        let handle = std::thread::spawn(move || {
            let mut sys = System::new();
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(rss_bytes(&mut sys), Ordering::Relaxed);
                pb.fetch_max(browser_rss_bytes(&mut sys), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        Self {
            stop,
            peak,
            peak_browser,
            handle: Some(handle),
        }
    }

    /// (peak driver RSS, peak browser RSS)
    fn stop(mut self) -> (u64, u64) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        (
            self.peak.load(Ordering::Relaxed),
            self.peak_browser.load(Ordering::Relaxed),
        )
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cdp = match std::env::var("CDP_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            println!(
                "CDP_URL is not set, so there is no browser to soak.\n\
                 Start one, then re-run:\n  \
                 chrome --headless=new --remote-debugging-port=9222 --user-data-dir=profiles/probe/soak about:blank\n  \
                 $env:CDP_URL='http://127.0.0.1:9222'; cargo run --release --example soak"
            );
            return Ok(());
        }
    };

    let rows = env_usize("SOAK_URLS", 200);
    let levels = env_levels("SOAK_LEVELS", &[1, 4, 8, 16, 32]);
    let min_title = std::env::var("SOAK_MIN_TITLE").unwrap_or_else(|_| "soak".into());
    let flaky = std::env::var("SOAK_FLAKY").is_ok_and(|v| v == "1");

    let port = serve(flaky);
    let urls: Vec<String> = (0..rows)
        .map(|i| format!("http://127.0.0.1:{port}/page?i={i}"))
        .collect();

    let mut sys = System::new();
    sys.refresh_memory();
    let (total, available) = (sys.total_memory(), sys.available_memory());
    println!(
        "soak: {} rows, levels {:?}, mem {:.0} MB total / {:.0} MB available, flaky {}",
        rows,
        levels,
        mb(total),
        mb(available),
        if flaky {
            "on (500 every 3rd, 429 every 7th)"
        } else {
            "off"
        }
    );
    println!("cdp: {cdp}");
    println!();

    let browser = CdpBackend::connect_with_url(&cdp).await?;
    let baseline = rss_bytes(&mut sys);
    let browser_base = browser_rss_bytes(&mut sys);
    println!(
        "baseline rss: {:.0} MB, browser processes: {:.0} MB\n",
        mb(baseline),
        mb(browser_base)
    );

    println!(
        "| tabs | urls | ok | err | wall ms | rows/s | driver peak MB | browser delta MB | delta/row KB | loaded |"
    );
    println!("| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | :--- |");

    let mut any_bad = false;
    for level in levels {
        let sampler = Sampler::start();
        let t0 = Instant::now();
        let results = batch_fetch(&browser, &urls, level, &NavOpts::default()).await;
        let wall = t0.elapsed();
        let (peak, peak_browser) = sampler.stop();

        let ok = results
            .iter()
            .filter(|r| r.error.is_none() && r.title.contains(&min_title))
            .count();
        let err = results.len().saturating_sub(ok);
        let delta_kb = if rows > 0 {
            (peak.saturating_sub(baseline)) as f64 / 1024.0 / rows as f64
        } else {
            0.0
        };
        if err > 0 {
            any_bad = true;
        }
        println!(
            "| {} | {} | {} | {} | {} | {:.1} | {:.0} | {:.0} | {:.1} | {} |",
            level,
            rows,
            ok,
            err,
            wall.as_millis(),
            ok as f64 / wall.as_secs_f64().max(0.001),
            mb(peak),
            mb(peak_browser.saturating_sub(browser_base)),
            delta_kb,
            if err == 0 { "yes" } else { "NO" }
        );

        sys.refresh_memory();
        println!(
            "  cached batch_tabs {:?}, available {:.0} MB",
            browser.batch_tabs(),
            mb(sys.available_memory())
        );
    }

    println!();
    if any_bad {
        println!("one or more levels returned rows that did not load the fixture");
    }
    println!("peak rss is process-wide, so the highest level is the one to compare across runs");
    Ok(())
}
