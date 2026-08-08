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

/// Chrome binary. Override with WEBRAIN_CHROME. Auto-detects the platform's
/// usual Chrome/Chromium/Edge install + PATH lookup (macOS/Linux).
///
/// Order: real Chrome FIRST (patchright's #1 best practice — CfT/Chromium is
/// more fingerprintable and Cloudflare withholds Turnstile tokens from it),
/// then CfT (`webrain install chrome`) as fallback.
pub fn chrome_path() -> PathBuf {
    if let Some(p) = std::env::var_os("WEBRAIN_CHROME") {
        return PathBuf::from(p);
    }
    #[cfg(target_os = "windows")]
    {
        for cand in [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        ] {
            let p = PathBuf::from(cand);
            if p.exists() {
                return p;
            }
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let p = PathBuf::from(local).join(r"Google\Chrome\Application\chrome.exe");
            if p.exists() {
                return p;
            }
        }
    }
    // CfT fallback only when no real Chrome/Edge is installed.
    if let Some(p) = crate::install::find_cft_chrome() {
        return p;
    }
    #[cfg(target_os = "macos")]
    {
        for cand in [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ] {
            let p = PathBuf::from(cand);
            if p.exists() {
                return p;
            }
        }
    }
    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
        "msedge",
        "brave-browser",
    ] {
        if let Ok(p) = which(name) {
            return p;
        }
    }
    // Last resort — spawn fails with a clear "failed to spawn Chrome" error.
    PathBuf::from("google-chrome")
}

pub(crate) fn which(name: &str) -> std::io::Result<PathBuf> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no PATH"))?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "not found",
    ))
}

/// Per-account profile root (platform-idiomatic data dirs). Override with
/// WEBRAIN_PROFILES_DIR.
pub fn profiles_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("WEBRAIN_PROFILES_DIR") {
        return PathBuf::from(p);
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(app) = std::env::var("APPDATA") {
            return PathBuf::from(app).join("webrain").join("profiles");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join("Library/Application Support/webrain/profiles");
        }
    }
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(x).join("webrain").join("profiles");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/webrain/profiles");
    }
    PathBuf::from("webrain/profiles")
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

/// Shared engine spawn: spawn `bin`, wait up to 20s for the CDP port, kill on
/// drop. ponytail: one wait loop for Chrome/lightpanda/obscura.
fn spawn_and_wait(
    bin: &std::path::Path,
    args: &[String],
    port: u16,
    cdp_url: String,
    profile_dir: PathBuf,
    name: &str,
) -> anyhow::Result<Launched> {
    let child = std::process::Command::new(bin)
        .args(args)
        .spawn()
        .with_context(|| format!("failed to spawn {name} at {}", bin.display()))?;
    let launched = Launched {
        port,
        cdp_url,
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
    anyhow::bail!("{name} did not open CDP on port {port} within 20s")
}

/// Spawn real Chrome (headed by default) with a persistent per-account profile
/// and stealth flags, wait for its CDP endpoint, and return a handle + CDP URL.
/// Chrome locks `user-data-dir` — one instance per profile/port.
pub fn launch_chrome(
    service: &str,
    profile: &str,
    port: u16,
    headed: bool,
) -> anyhow::Result<Launched> {
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
    args.push("about:blank".to_string());
    spawn_and_wait(
        &chrome_path(),
        &args,
        port,
        format!("http://127.0.0.1:{port}"),
        profile_dir,
        "Chrome",
    )
}

/// Spawn the lightpanda CDP server (agent-browser `--engine lightpanda`).
/// `lightpanda serve` exposes raw CDP on the port; CdpBackend connects via
/// ws:// (resolve_ws passes it through). Binary from install::find_lightpanda()
/// (PATH, ~/.lightpanda, ~/.local/bin, or WEBRAIN_LIGHTPANDA).
/// ponytail: reuse `Launched` (kills on drop); no profile dir for lightpanda.
pub fn launch_lightpanda(port: u16) -> anyhow::Result<Launched> {
    if port_open(port) {
        anyhow::bail!("port {port} already has a CDP endpoint — another server is running there");
    }
    let bin = crate::install::find_lightpanda().ok_or_else(|| {
        anyhow::anyhow!(
            "lightpanda not found — install the binary (see docs/AGENT_DECISION_GUIDE.md) or set WEBRAIN_LIGHTPANDA"
        )
    })?;
    let args = vec![
        "serve".to_string(),
        "--host".to_string(),
        "0.0.0.0".to_string(),
        "--port".to_string(),
        port.to_string(),
        "--advertise-host".to_string(),
        "127.0.0.1".to_string(),
    ];
    spawn_and_wait(
        &bin,
        &args,
        port,
        format!("ws://127.0.0.1:{port}"),
        PathBuf::new(),
        "lightpanda",
    )
}

/// Spawn the Obscura CDP server (agent-browser-style engine). Binary from
/// install::find_obscura() (`webrain install --engine obscura`, PATH,
/// WEBRAIN_OBSCURA). CDP endpoint mirrors Chrome's /devtools/browser path.
/// ponytail: reuse `Launched`; no profile dir for obscura.
pub fn launch_obscura(port: u16) -> anyhow::Result<Launched> {
    if port_open(port) {
        anyhow::bail!("port {port} already has a CDP endpoint — another server is running there");
    }
    let bin = crate::install::find_obscura().ok_or_else(|| {
        anyhow::anyhow!(
            "obscura not found — run `webrain install --engine obscura` or set WEBRAIN_OBSCURA"
        )
    })?;
    let args = vec![
        "serve".to_string(),
        "--host".to_string(),
        "0.0.0.0".to_string(),
        "--port".to_string(),
        port.to_string(),
        "--stealth".to_string(),
    ];
    spawn_and_wait(
        &bin,
        &args,
        port,
        format!("ws://127.0.0.1:{port}/devtools/browser"),
        PathBuf::new(),
        "obscura",
    )
}
