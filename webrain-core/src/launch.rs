// Native Chrome launcher — replaces the Python stealth_solve.py launch path.
// Spawns real Chrome with a persistent per-account profile + the same stealth
// flags the sidecar used, hands the CDP URL to CdpBackend (which applies
// STEALTH_JS + UA override on attach). chromiumoxide slots in here later for
// native login/2FA.
//
// ponytail: spawn the subprocess ourselves (keep exit control + exact flags);
// CdpBackend already drives CDP, so no second stack is needed to start.

use anyhow::Context;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Chrome binary. Override with WEBRAIN_CHROME.
pub fn chrome_path() -> PathBuf {
    std::env::var_os("WEBRAIN_CHROME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"))
}

/// Per-account profile root: `$WEBRAIN_PROFILES_DIR` or `%APPDATA%/webrain/profiles`.
pub fn profiles_dir() -> PathBuf {
    std::env::var_os("WEBRAIN_PROFILES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let base = std::env::var_os("APPDATA")
                .or_else(|| std::env::var_os("HOME"))
                .unwrap_or_else(|| ".".into());
            PathBuf::from(base).join("webrain").join("profiles")
        })
}

/// A launched Chrome process; kills Chrome on drop (no orphans on normal exit).
pub struct Launched {
    pub port: u16,
    pub cdp_url: String,
    pub profile_dir: PathBuf,
    child: std::process::Child,
}

impl Drop for Launched {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn port_open(port: u16) -> bool {
    TcpStream::connect(("127.0.0.1", port)).is_ok()
}

/// Spawn real Chrome (headed by default) with a persistent per-account profile
/// and stealth flags, wait for its CDP endpoint, and return a handle + CDP URL.
/// Chrome locks `user-data-dir` — one instance per profile/port.
pub fn launch_chrome(service: &str, profile: &str, port: u16, headed: bool) -> anyhow::Result<Launched> {
    if port_open(port) {
        anyhow::bail!("port {port} already has a CDP endpoint — another Chrome is running there");
    }
    let profile_dir = profiles_dir().join(service).join(profile);
    std::fs::create_dir_all(&profile_dir)?;

    let mut args = vec![
        format!("--remote-debugging-port={port}"),
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--user-data-dir={}", profile_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        // stealth flags — same set the Python sidecar used
        "--disable-blink-features=AutomationControlled".to_string(),
        "--disable-features=AutomationControlled".to_string(),
    ];
    if !headed {
        args.push("--headless=new".to_string());
    }
    let child = std::process::Command::new(chrome_path())
        .args(&args)
        .arg("about:blank")
        .spawn()
        .with_context(|| format!("failed to spawn Chrome at {}", chrome_path().display()))?;

    let launched = Launched {
        port,
        cdp_url: format!("http://127.0.0.1:{port}"),
        profile_dir,
        child,
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if port_open(port) {
            return Ok(launched);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    anyhow::bail!("Chrome did not open CDP on port {port} within 20s")
}
