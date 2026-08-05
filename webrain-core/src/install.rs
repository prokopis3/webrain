// Engine install + discovery — mirrors vercel-labs/agent-browser:
//   - `webrain install` downloads Chrome for Testing into a cache dir
//     (agent-browser: last-known-good JSON -> platform zip -> cache).
//   - `find_cft_chrome()` lets launch::chrome_path() prefer the downloaded
//     build over system Chrome (agent-browser `find_chrome`).
//   - `find_lightpanda()` discovers the lightpanda binary for
//     launch::launch_lightpanda() (agent-browser `find_lightpanda`).
//
// ponytail: ureq (already a dep) instead of reqwest; CfT archives are .zip on
// every platform, so the `zip` crate covers win/mac/linux — no tar needed.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

const CFG_JSON_URL: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";

/// Engine cache root. Override with WEBRAIN_BROWSERS_DIR.
pub fn browsers_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("WEBRAIN_BROWSERS_DIR") {
        return PathBuf::from(p);
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("webrain").join("browsers");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join("Library/Caches/webrain/browsers");
        }
    }
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(x).join("webrain").join("browsers");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache/webrain/browsers");
    }
    PathBuf::from("webrain/browsers")
}

/// Chrome-for-Testing platform key, e.g. "win64", "mac-arm64", "linux64".
pub fn platform_key() -> &'static str {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "x86_64"
    };
    match (os, arch) {
        ("windows", "x86_64") => "win64",
        ("windows", "x86") => "win32",
        ("macos", "arm64") => "mac-arm64",
        ("macos", "x86_64") => "mac-x64",
        ("linux", "x86_64") => "linux64",
        ("linux", "arm64") => "linux-arm64",
        // Fallback: install fails with a clear "no chrome download" error.
        _ => "linux64",
    }
}

fn bin_name(base: &str) -> String {
    #[cfg(windows)]
    {
        format!("{base}.exe")
    }
    #[cfg(not(windows))]
    {
        base.to_string()
    }
}

/// Depth-limited recursive search for a file by exact name (CfT zip layout is
/// shallow: `chrome-win64/chrome.exe`, `.../Google Chrome for Testing.app/...`).
fn find_named(root: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(root).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_file() && p.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(p);
        }
        if p.is_dir() {
            if let Some(f) = find_named(&p, name, depth - 1) {
                return Some(f);
            }
        }
    }
    None
}

/// Chrome downloaded by `webrain install`, newest `chrome-<version>` first.
pub fn find_cft_chrome() -> Option<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(browsers_dir()).ok()?.flatten().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
    for e in entries {
        let p = e.path();
        if p.is_dir() && e.file_name().to_string_lossy().starts_with("chrome-") {
            if let Some(b) = find_named(&p, &bin_name("chrome"), 4) {
                return Some(b);
            }
        }
    }
    None
}

/// lightpanda binary: WEBRAIN_LIGHTPANDA, then PATH, then agent-browser's
/// home candidates (~/.lightpanda, ~/.local/bin), then the
/// `webrain install --engine lightpanda` cache (lightpanda-<tag>/).
pub fn find_lightpanda() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WEBRAIN_LIGHTPANDA") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(p) = crate::launch::which("lightpanda") {
        return Some(p);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for cand in [
            home.join(".lightpanda/lightpanda"),
            home.join(".local/bin/lightpanda"),
        ] {
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    let mut entries: Vec<_> = std::fs::read_dir(browsers_dir()).ok()?.flatten().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
    for e in entries {
        let p = e.path();
        if p.is_dir() && e.file_name().to_string_lossy().starts_with("lightpanda-") {
            if let Some(b) = find_named(&p, &bin_name("lightpanda"), 1) {
                return Some(b);
            }
        }
    }
    None
}

/// Obscura binary: WEBRAIN_OBSCURA, PATH, ~/.obscura / ~/.local/bin, then the
/// `webrain install --engine obscura` cache (obscura-<tag>/).
pub fn find_obscura() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WEBRAIN_OBSCURA") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(p) = crate::launch::which("obscura") {
        return Some(p);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for cand in [
            home.join(".obscura/obscura"),
            home.join(".local/bin/obscura"),
        ] {
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    let mut entries: Vec<_> = std::fs::read_dir(browsers_dir()).ok()?.flatten().collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
    for e in entries {
        let p = e.path();
        if p.is_dir() && e.file_name().to_string_lossy().starts_with("obscura-") {
            if let Some(b) = find_named(&p, &bin_name("obscura"), 3) {
                return Some(b);
            }
        }
    }
    None
}

/// Obscura release asset key, e.g. "x86_64-windows", "aarch64-linux"
/// (obscura ships only x86_64-windows).
fn obscura_asset_key() -> &'static str {
    if cfg!(target_os = "windows") {
        "x86_64-windows"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-macos"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-macos"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-linux"
    } else {
        "x86_64-linux"
    }
}

const OBSCURA_RELEASES_URL: &str =
    "https://api.github.com/repos/h4ckf0r0day/obscura/releases/latest";

/// Download the latest Obscura release (agent-browser-style: fetch a known
/// release, extract into the engine cache). `--stealth` picks the BoringSSL
/// stealth build. Windows assets are .zip; linux/macos are .tar.gz.
pub fn install_obscura(force: bool, stealth: bool) -> Result<PathBuf> {
    if !force {
        if let Some(bin) = find_obscura() {
            println!("Obscura already installed: {}", bin.display());
            return Ok(bin);
        }
    }
    let dir = browsers_dir();
    std::fs::create_dir_all(&dir)?;

    let raw = String::from_utf8(download_bytes(OBSCURA_RELEASES_URL)?)
        .context("obscura releases JSON is not UTF-8")?;
    let rel: Value = serde_json::from_str(&raw).context("parse obscura releases JSON")?;
    let tag = rel["tag_name"]
        .as_str()
        .context("no tag_name in obscura release")?;
    let key = obscura_asset_key();
    let asset = rel["assets"]
        .as_array()
        .context("no assets in obscura release")?
        .iter()
        .find(|a| {
            let n = a["name"].as_str().unwrap_or("");
            n.contains("obscura-") && n.contains(key) && n.contains("stealth") == stealth
        })
        .and_then(|a| a["browser_download_url"].as_str())
        .context("no matching obscura asset for this platform")?;

    let fname = asset.rsplit('/').next().unwrap_or("archive").to_string();
    println!("Downloading Obscura {tag} ({fname})...");
    let bytes = download_bytes(asset)?;
    let dest = dir.join(format!("obscura-{tag}"));
    extract_archive(&bytes, &dest, &fname)?;
    find_named(&dest, &bin_name("obscura"), 3)
        .with_context(|| format!("obscura binary not found in {}", dest.display()))
}

/// lightpanda asset suffix, e.g. "x86_64-linux". lightpanda ships NO Windows
/// build — on Windows there is nothing to download (use the Docker image
/// lightpanda/browser:nightly instead).
fn lightpanda_asset_key() -> Option<&'static str> {
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    if cfg!(target_os = "linux") {
        Some(match arch {
            "aarch64" => "aarch64-linux",
            _ => "x86_64-linux",
        })
    } else if cfg!(target_os = "macos") {
        Some(match arch {
            "aarch64" => "aarch64-macos",
            _ => "x86_64-macos",
        })
    } else {
        None
    }
}

const LIGHTPANDA_RELEASES_URL: &str =
    "https://api.github.com/repos/lightpanda-io/browser/releases/latest";

/// Download the latest lightpanda binary (raw asset, no archive — unlike
/// obscura). Mirrors install_obscura: cache under browsers_dir(), chmod +x on
/// unix. On Windows it bails — lightpanda publishes no Windows asset.
/// ponytail: find_named depth 1, the binary lands directly in the cache dir.
pub fn install_lightpanda(force: bool) -> Result<PathBuf> {
    if !force {
        if let Some(bin) = find_lightpanda() {
            println!("lightpanda already installed: {}", bin.display());
            return Ok(bin);
        }
    }
    let key = lightpanda_asset_key().ok_or_else(|| {
        anyhow::anyhow!(
            "lightpanda ships no {} binary — use the Docker image (lightpanda/browser:nightly) or a linux/macos host",
            if cfg!(target_os = "windows") {
                "Windows"
            } else {
                "this-platform"
            }
        )
    })?;
    let dir = browsers_dir();
    std::fs::create_dir_all(&dir)?;

    let raw = String::from_utf8(download_bytes(LIGHTPANDA_RELEASES_URL)?)
        .context("lightpanda releases JSON is not UTF-8")?;
    let rel: Value = serde_json::from_str(&raw).context("parse lightpanda releases JSON")?;
    let tag = rel["tag_name"]
        .as_str()
        .context("no tag_name in lightpanda release")?;
    let want = format!("lightpanda-{key}");
    let asset = rel["assets"]
        .as_array()
        .context("no assets in lightpanda release")?
        .iter()
        .find(|a| a["name"].as_str().unwrap_or("") == want)
        .and_then(|a| a["browser_download_url"].as_str())
        .context("no matching lightpanda asset for this platform")?;

    println!("Downloading lightpanda {tag} ({key})...");
    let bytes = download_bytes(asset)?;
    let dest = dir.join(format!("lightpanda-{tag}"));
    std::fs::create_dir_all(&dest)?;
    let out = dest.join(bin_name("lightpanda"));
    std::fs::write(&out, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(out)
}

fn download_bytes(url: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    // ureq 3: read_to_vec() caps at 10 MB, too small for the engine zips.
    // into_with_config() defaults to unlimited (u64::MAX).
    Ok(resp.into_body().into_with_config().read_to_vec()?)
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let name = f.name().to_string();
        let out = dest.join(&name);
        if f.is_dir() || name.ends_with('/') {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut f, &mut w)?;
    }
    Ok(())
}

/// .zip via the zip crate; .tar.gz via system `tar` (ships on linux/macos) —
/// no tar crate needed.
fn extract_archive(bytes: &[u8], dest: &Path, fname: &str) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    if fname.ends_with(".zip") {
        return extract_zip(bytes, dest);
    }
    let tmp = dest.join("archive.tar.gz");
    std::fs::write(&tmp, bytes)?;
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(&tmp)
        .arg("-C")
        .arg(dest)
        .status()?;
    let _ = std::fs::remove_file(&tmp);
    if !status.success() {
        anyhow::bail!("tar -xzf failed for {fname}");
    }
    Ok(())
}

/// Download Chrome for Testing (agent-browser `install`). Skips if already
/// present unless `force`. Returns the binary path.
pub fn install_chrome(force: bool) -> Result<PathBuf> {
    if !force {
        if let Some(bin) = find_cft_chrome() {
            println!("Chrome for Testing already installed: {}", bin.display());
            return Ok(bin);
        }
    }
    let dir = browsers_dir();
    std::fs::create_dir_all(&dir)?;

    let raw = String::from_utf8(download_bytes(CFG_JSON_URL)?).context("CfT JSON is not UTF-8")?;
    let body: Value = serde_json::from_str(&raw).context("parse Chrome for Testing JSON")?;
    let version = body["channels"]["Stable"]["version"]
        .as_str()
        .context("no Stable version in CfT JSON")?;
    let url = body["channels"]["Stable"]["downloads"]["chrome"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|d| d["platform"].as_str() == Some(platform_key()))
        })
        .and_then(|d| d["url"].as_str())
        .context("no chrome download for this platform (cf. platform_key())")?;

    println!(
        "Downloading Chrome for Testing {version} ({})...",
        platform_key()
    );
    println!("  {url}");
    let bytes = download_bytes(url)?;
    let dest = dir.join(format!("chrome-{version}"));
    extract_zip(&bytes, &dest)?;
    find_named(&dest, &bin_name("chrome"), 4).with_context(|| {
        format!(
            "chrome binary not found after extract in {}",
            dest.display()
        )
    })
}

#[cfg(test)]
mod tests {
    // ponytail: one self-check for the non-trivial recursive binary finder.
    #[test]
    fn find_named_walks_nested() {
        let dir = std::env::temp_dir().join(format!("webrain-inst-{}", std::process::id()));
        let bin = dir.join("chrome-win64/chrome.exe");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"").unwrap();
        let found = super::find_named(&dir, "chrome.exe", 4);
        assert_eq!(found, Some(bin));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
