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
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const CFG_JSON_URL: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
const LLAMA_RELEASES_API: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest";
const VISION_HF: &str = "https://huggingface.co/unsloth/Qwen3-VL-2B-Instruct-GGUF/resolve/main";
const VISION_MODEL: &str = "Qwen3-VL-2B-Instruct-Q4_K_M.gguf";
const VISION_MMPROJ: &str = "mmproj-F16.gguf";

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

/// First numeric component of a `chrome-<ver>` / `lightpanda-v<tag>` /
/// `obscura-v<tag>` cache dir name — for NUMERIC version ordering (newest
/// build first), not lexicographic: `chrome-99` must sort below `chrome-100`.
fn dir_version(name: &std::ffi::OsStr) -> u64 {
    name.to_string_lossy()
        .split(|c: char| !c.is_ascii_digit())
        .find_map(|tok| tok.parse::<u64>().ok())
        .unwrap_or(0)
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
    entries.sort_by(|a, b| dir_version(&b.file_name()).cmp(&dir_version(&a.file_name())));
    for e in entries {
        let p = e.path();
        if p.is_dir() && e.file_name().to_string_lossy().starts_with("chrome-") {
            // macOS ships the executable as "Google Chrome for Testing" inside the
            // .app bundle (not a bare `chrome`), so look for the platform name.
            let exe = if cfg!(target_os = "macos") {
                "Google Chrome for Testing".to_string()
            } else {
                bin_name("chrome")
            };
            if let Some(b) = find_named(&p, &exe, 4) {
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
    entries.sort_by(|a, b| dir_version(&b.file_name()).cmp(&dir_version(&a.file_name())));
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
    entries.sort_by(|a, b| dir_version(&b.file_name()).cmp(&dir_version(&a.file_name())));
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
/// stealth build, `--no-render` the headless no-render build. Windows assets
/// are .zip; linux/macos are .tar.gz.
///
/// v0.2.0 ships 4 packages per platform — exact suffix match, no heuristic:
///   render+stealth   -> obscura-<key>-stealth.<ext>
///   render (default) -> obscura-<key>.<ext>
///   no-render+stealth-> obscura-<key>-no-render-stealth.<ext>
///   no-render        -> obscura-<key>-no-render.<ext>
/// v0.1.11 fallback only has the plain (render, no-stealth) package.
pub fn install_obscura(force: bool, stealth: bool, render: bool) -> Result<PathBuf> {
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
    // Try /latest first. If no matching asset (new release missing platform
    // binaries, or v0.2.0 lacks the requested variant), fall back to v0.1.11.
    // ponytail: one retry, not a loop.
    fn find_asset<'a>(rel: &'a Value, key: &str, stealth: bool, render: bool) -> Option<&'a str> {
        let suffix = match (render, stealth) {
            (true, false) => "",
            (true, true) => "-stealth",
            (false, false) => "-no-render",
            (false, true) => "-no-render-stealth",
        };
        let prefix = format!("obscura-{key}{suffix}");
        rel["assets"].as_array()?.iter().find_map(|a| {
            let n = a["name"].as_str().unwrap_or("");
            let rest = n.strip_prefix(&prefix)?;
            if rest.is_empty() || rest == ".tar.gz" || rest == ".zip" {
                a["browser_download_url"].as_str()
            } else {
                None
            }
        })
    }
    let (asset_url, tag) = match find_asset(&rel, key, stealth, render) {
        Some(url) => (url.to_string(), tag.to_string()),
        None => {
            let fallback_url = OBSCURA_RELEASES_URL.replace("/latest", "/tags/v0.1.11");
            let raw2 = String::from_utf8(download_bytes(&fallback_url)?)
                .context("obscura fallback release JSON")?;
            let rel2: Value = serde_json::from_str(&raw2).context("parse fallback release")?;
            let url = find_asset(&rel2, key, stealth, render)
                .context("no matching obscura asset (tried latest + v0.1.11)")?;
            let t = rel2["tag_name"].as_str().unwrap_or("v0.1.11");
            (url.to_string(), t.to_string())
        }
    };

    let fname = asset_url
        .rsplit('/')
        .next()
        .unwrap_or("archive")
        .to_string();
    println!("Downloading Obscura {tag} ({fname})...");
    let bytes = download_bytes(&asset_url)?;
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

/// whisper.cpp GGUF model names `webrain install whisper` accepts.
const WHISPER_MODELS: &[&str] = &[
    "tiny.en",
    "tiny",
    "base.en",
    "base",
    "small.en",
    "small",
    "medium.en",
    "medium",
    "large-v1",
    "large-v2",
    "large-v3",
    "large-v3-turbo",
];

/// Download a whisper.cpp GGUF model into `<browsers_dir>/whisper/` for the
/// local `webrain watch` backend. The `whisper-cli` binary stays PATH/env
/// (like yt-dlp/ffmpeg — no reliable cross-OS prebuilt we can ship); the model
/// is the 100MB-1.5GB part worth managing here.
/// ponytail: whitelist instead of arbitrary paths — no URL/path injection.
pub fn install_whisper_model(model: &str, force: bool) -> Result<PathBuf> {
    let model = model.trim();
    if !WHISPER_MODELS.contains(&model) {
        anyhow::bail!(
            "unknown model `{model}` — pick one of: {}",
            WHISPER_MODELS.join(", ")
        );
    }
    let dir = browsers_dir().join("whisper");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("ggml-{model}.bin"));
    if !force && dest.exists() {
        println!("whisper model already installed: {}", dest.display());
        return Ok(dest);
    }
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{model}.bin");
    println!("Downloading whisper model {model} (~{})...", dest.display());
    let bytes = download_bytes(&url)?;
    std::fs::write(&dest, &bytes)?;
    println!("whisper model ready: {}", dest.display());
    Ok(dest)
}

/// Download `url` into memory via the parallel chunked `download_to_file` (so
/// every `install_*` — obscura, chrome, ffmpeg, whisper, yt-dlp — gets the same
/// animated progress bar + honest per-chunk completion as the vision model).
/// Streams to a temp file in the cache dir, reads it back, removes it.
fn download_bytes(url: &str) -> Result<Vec<u8>> {
    // Unique per-call temp name (pid + monotonic nonce): concurrent installs in
    // one process no longer collide, a stale sidecar from a *different* URL can't
    // be resumed, and a predictable PID-only path can't be symlink-attacked.
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "webrain-dl-{}-{}",
        std::process::id(),
        NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&tmp);
    download_to_file(url, &tmp)?;
    let bytes = std::fs::read(&tmp)?;
    // download_to_file leaves .part/.part.done/.ok sidecars — clean them all.
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(tmp.with_extension("part"));
    let _ = std::fs::remove_file(tmp.with_extension("part.done"));
    let _ = std::fs::remove_file(tmp.with_extension("ok"));
    Ok(bytes)
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let raw = f.name().replace('\\', "/");
        // Zip-slip guard: reject absolute paths and any `..` traversal component
        // before joining onto `dest` (a crafted entry must not escape the dir).
        if raw.starts_with('/') || raw.split('/').any(|c| c == "..") {
            anyhow::bail!("unsafe zip entry name: {raw:?}");
        }
        let out = dest.join(&raw);
        if f.is_dir() || raw.ends_with('/') {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut f, &mut w)?;
        #[cfg(unix)]
        if let Some(mode) = f.unix_mode() {
            // Preserve the archive's mode — CfT/whisper archives ship
            // `chrome`/`whisper-cli` as 0755; File::create alone (0644 & ~umask)
            // would leave them non-executable on Linux/macOS.
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

/// .zip via the zip crate; .tar.gz / .tar.xz via system `tar` (ships on all OS).
fn extract_archive(bytes: &[u8], dest: &Path, fname: &str) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    if fname.ends_with(".zip") {
        return extract_zip(bytes, dest);
    }
    let (flags, tmp) = if fname.ends_with(".tar.xz") {
        ("-xJf", dest.join("archive.tar.xz"))
    } else {
        ("-xzf", dest.join("archive.tar.gz"))
    };
    std::fs::write(&tmp, bytes)?;
    let status = std::process::Command::new("tar")
        .arg(flags)
        .arg(&tmp)
        .arg("-C")
        .arg(dest)
        .status()?;
    let _ = std::fs::remove_file(&tmp);
    if !status.success() {
        anyhow::bail!("tar {flags} failed for {fname}");
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
    // macOS ships "Google Chrome for Testing.app/Contents/MacOS/Google Chrome
    // for Testing" (not a bare `chrome`), so search for the platform exe name.
    let exe = if cfg!(target_os = "macos") {
        "Google Chrome for Testing".to_string()
    } else {
        bin_name("chrome")
    };
    find_named(&dest, &exe, 4).with_context(|| {
        format!(
            "chrome binary not found after extract in {}",
            dest.display()
        )
    })
}

// ---------------------------------------------------------------------------
// Watch bundle — self-contained `webrain watch` (mono packages, all OS).
// `webrain install watch` downloads ffmpeg(+ffprobe), yt-dlp, whisper-cli and
// a GGUF model into <browsers_dir>/tools/. watch auto-resolves bundled →
// WEBRAIN_*_BIN → PATH, so nothing has to be installed on the system PATH.
// ---------------------------------------------------------------------------

const BTBN_BASE: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest";
const YTDLP_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
const WHISPER_BIN_BASE: &str = "https://github.com/ggml-org/whisper.cpp/releases/latest/download";

/// Resolve a tool binary: <browsers_dir>/tools/<tool>/<bin> (bundled mono
/// package) else PATH. `webrain watch` uses this for ffmpeg/ffprobe/yt-dlp/
/// whisper-cli so the bundle is self-contained — no system install needed.
pub fn find_tool(tool: &str) -> Option<PathBuf> {
    let exe = bin_name(tool);
    let base = browsers_dir().join("tools");
    let own = base.join(tool).join(&exe);
    if own.is_file() {
        return Some(own);
    }
    // yt-dlp installs as yt-dlp_linux / yt-dlp_macos / yt-dlp.exe (not the
    // bare name) — check the platform variants too so the bundled binary is
    // found on every OS. ponytail: one match, mirror of install_ytdlp's names.
    let exe_variant = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => None,
        ("linux", "aarch64") => Some("yt-dlp_linux_aarch64".to_string()),
        ("linux", _) => Some("yt-dlp_linux".to_string()),
        ("macos", _) => Some("yt-dlp_macos".to_string()),
        _ => None,
    };
    if tool == "yt-dlp" {
        if let Some(v) = exe_variant {
            let own_v = base.join(tool).join(&v);
            if own_v.is_file() {
                return Some(own_v);
            }
        }
    }
    // ffprobe rides in the ffmpeg dir (shared av DLLs).
    if tool == "ffprobe" {
        let in_ffmpeg = base.join("ffmpeg").join(&exe);
        if in_ffmpeg.is_file() {
            return Some(in_ffmpeg);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|d| {
            let f = d.join(&exe);
            f.is_file().then_some(f)
        })
    })
}

fn make_executable(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    {
        let _ = p;
    }
}

/// Install yt-dlp (single-file binary) into tools/yt-dlp/.
fn install_ytdlp(force: bool) -> Result<PathBuf> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let (name, url) = match (os, arch) {
        ("windows", _) => ("yt-dlp.exe", format!("{YTDLP_BASE}/yt-dlp.exe")),
        ("linux", "aarch64") => (
            "yt-dlp_linux_aarch64",
            format!("{YTDLP_BASE}/yt-dlp_linux_aarch64"),
        ),
        ("linux", _) => ("yt-dlp_linux", format!("{YTDLP_BASE}/yt-dlp_linux")),
        ("macos", _) => ("yt-dlp_macos", format!("{YTDLP_BASE}/yt-dlp_macos")),
        _ => anyhow::bail!("yt-dlp has no binary for {os}/{arch}"),
    };
    let dir = browsers_dir().join("tools").join("yt-dlp");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(name);
    if !force && dest.exists() {
        println!("yt-dlp already installed: {}", dest.display());
        return Ok(dest);
    }
    println!("Downloading yt-dlp...");
    let bytes = download_bytes(&url)?;
    std::fs::write(&dest, &bytes)?;
    make_executable(&dest);
    println!("yt-dlp ready: {}", dest.display());
    Ok(dest)
}

/// Install ffmpeg + ffprobe (BtbN shared builds) into tools/ffmpeg/ — exe and
/// sibling av DLLs copied flat so both binaries resolve there.
fn install_ffmpeg(force: bool) -> Result<(PathBuf, PathBuf)> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let asset = match (os, arch) {
        ("windows", _) => "win64-gpl.zip",
        ("linux", "aarch64") => "linuxarm64-gpl.tar.xz",
        ("linux", _) => "linux64-gpl.tar.xz",
        ("macos", "aarch64") => "macosarm64-gpl.tar.xz",
        ("macos", _) => "macos64-gpl.tar.xz",
        _ => anyhow::bail!("ffmpeg (BtbN) has no build for {os}/{arch}"),
    };
    let dir = browsers_dir().join("tools").join("ffmpeg");
    let ffmpeg = dir.join(bin_name("ffmpeg"));
    let ffprobe = dir.join(bin_name("ffprobe"));
    if !force && ffmpeg.exists() && ffprobe.exists() {
        println!("ffmpeg + ffprobe already installed");
        return Ok((ffmpeg, ffprobe));
    }
    println!("Downloading ffmpeg ({asset})...");
    let bytes = download_bytes(&format!("{BTBN_BASE}-{asset}"))?;
    let work = dir.join("_extract");
    let _ = std::fs::remove_dir_all(&work);
    extract_archive(&bytes, &work, asset)?;
    let ffmpeg_src = find_named(&work, &bin_name("ffmpeg"), 4)
        .with_context(|| "ffmpeg not found after extract")?;
    // Copy the whole bin/ dir flat — BtbN is a shared build (avcodec-*.dll etc.)
    // that ffprobe needs alongside ffmpeg.exe.
    let bin_dir = ffmpeg_src
        .parent()
        .context("ffmpeg extract dir")?
        .to_path_buf();
    std::fs::create_dir_all(&dir)?;
    for e in std::fs::read_dir(&bin_dir)? {
        let e = e?;
        let p = e.path();
        if p.is_file() {
            // Propagate: a failed DLL copy (Windows avcodec-*.dll) must not
            // silently produce a broken install reported as "ok".
            std::fs::copy(&p, dir.join(p.file_name().unwrap()))?;
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    make_executable(&ffmpeg);
    make_executable(&ffprobe);
    println!("ffmpeg ready: {}", ffmpeg.display());
    println!("ffprobe ready: {}", ffprobe.display());
    Ok((ffmpeg, ffprobe))
}

/// Install whisper-cli (whisper.cpp prebuilt) into tools/whisper/. macOS ships
/// no prebuilt — PATH (brew install whisper-cpp) is the fallback there.
fn install_whisper_bin(force: bool) -> Result<PathBuf> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let asset = match (os, arch) {
        ("windows", "x86_64") => "whisper-bin-x64.zip",
        ("windows", _) => "whisper-bin-Win32.zip",
        ("linux", "aarch64") => "whisper-bin-ubuntu-arm64.tar.gz",
        ("linux", _) => "whisper-bin-ubuntu-x64.tar.gz",
        _ => anyhow::bail!(
            "whisper.cpp ships no {} prebuilt — install whisper-cli via PATH (macOS: `brew install whisper-cpp`) or set WEBRAIN_WHISPER_BIN",
            if os == "macos" {
                "macOS"
            } else {
                "this-platform"
            }
        ),
    };
    let dir = browsers_dir().join("tools").join("whisper-cli");
    let dest = dir.join(bin_name("whisper-cli"));
    if !force && dest.exists() {
        println!("whisper-cli already installed: {}", dest.display());
        return Ok(dest);
    }
    println!("Downloading whisper-cli ({asset})...");
    let bytes = download_bytes(&format!("{WHISPER_BIN_BASE}/{asset}"))?;
    let work = dir.join("_extract");
    let _ = std::fs::remove_dir_all(&work);
    extract_archive(&bytes, &work, asset)?;
    let src = find_named(&work, &bin_name("whisper-cli"), 4)
        .with_context(|| "whisper-cli not found after extract")?;
    // Copy the whole release dir flat — whisper-cli loads whisper.dll / ggml.dll
    // / a ggml-cpu-*.dll backend from its OWN directory at runtime.
    let src_dir = src
        .parent()
        .context("whisper-cli extract dir")?
        .to_path_buf();
    std::fs::create_dir_all(&dir)?;
    for e in std::fs::read_dir(&src_dir)? {
        let e = e?;
        let p = e.path();
        if p.is_file() {
            // Propagate: whisper.dll/ggml-cpu-*.dll must land or the install fails.
            std::fs::copy(&p, dir.join(p.file_name().unwrap()))?;
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    make_executable(&dest);
    println!("whisper-cli ready: {}", dest.display());
    Ok(dest)
}

/// Format a byte count as human-readable (B/KB/MB/GB).
fn human_bytes(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{v:.0} {}", units[u])
    } else {
        format!("{v:.1} {}", units[u])
    }
}

/// Fallback for servers that ignore Range (GitHub API JSON etc.): one plain GET
/// streamed straight to `dest`. No chunking, no sidecar — small responses.
fn download_plain(url: &str, dest: &Path) -> Result<()> {
    use std::io::{Read, Write};
    let resp = ureq::get(url)
        .header("User-Agent", "webrain")
        .call()
        .with_context(|| format!("GET {url}"))?;
    let total = resp
        .headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let name = url.rsplit('/').next().unwrap_or(url).to_string();
    let mut body = resp.into_body().into_reader();
    let mut file = std::fs::File::create(dest)?;
    let mut done: u64 = 0;
    let mut last: i64 = -1;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        done += n as u64;
        file.write_all(&buf[..n])?;
        let pct = total
            .filter(|t| *t > 0)
            .map(|t| ((done as f64 / t as f64) * 100.0) as i64)
            .unwrap_or(-1);
        if pct != last {
            match total {
                Some(t) if t > 0 => print!("\r  {done} / {t} bytes ({pct}%)   "),
                _ => print!("\r  {:.1} MiB   ", done as f64 / 1048576.0),
            }
            let _ = std::io::stdout().flush();
            last = pct;
        }
    }
    print!(
        "\r  {name}: {:.1} MiB                \n",
        done as f64 / 1048576.0
    );
    let _ = std::io::stdout().flush();
    Ok(())
}

/// Download a URL to `dest` in **parallel Range chunks** (the vision model is
/// ~1.8 GiB; a single connection to HF/CDN throttles to ~KB/s, N parallel GETs
/// multiply throughput). Downloads to `<dest>.part` then renames on success.
///
/// **Completion is tracked per-chunk, not by file size** — pre-allocating the
/// `.part` (set_len) makes its length == total even when empty, so a length
/// check would "resume" a file of zeros. Each finished chunk appends its index
/// to `<dest>.part.done`; the file is renamed ONLY when every chunk is marked,
/// and a `<dest>.ok` marker (not size) proves a dest is genuinely complete.
/// A killed run therefore resumes the missing chunks instead of faking success.
fn download_to_file(url: &str, dest: &Path) -> Result<()> {
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    let part = dest.with_extension("part");
    let sidecar = dest.with_extension("part.done");
    let ok = dest.with_extension("ok");
    let name = url.rsplit('/').next().unwrap_or(url).to_string();

    // Probe total size with a 1-byte Range (CDN answers 206 + Content-Range).
    let probe = ureq::get(url)
        .header("User-Agent", "webrain")
        .header("Range", "bytes=0-0")
        .call()
        .with_context(|| format!("probe {url}"))?;
    let total: Option<u64> = if probe.status() == 206 {
        probe
            .headers()
            .get("Content-Range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|v| v.parse().ok())
    } else {
        None
    };

    // Server ignored Range (e.g. GitHub API JSON) — plain single-stream copy.
    let Some(total) = total else {
        return download_plain(url, dest);
    };

    // Genuinely complete (has the .ok marker AND the destination file — a stale
    // marker left behind after the dest was deleted must not fake success).
    if ok.exists() && dest.exists() {
        println!("  ✓ {name}: {} (already complete)", human_bytes(total));
        return Ok(());
    }

    let workers = 8usize.min(total.div_ceil(16 * 1024 * 1024) as usize).max(1);
    let chunk = total.div_ceil(workers as u64);
    let done = Arc::new(AtomicU64::new(0));
    let completed = Arc::new(Mutex::new(std::collections::HashSet::<u64>::new()));
    // Counts workers that have RETURNED (success or failure) so the progress
    // loop can terminate even when done never reaches total (a failed chunk
    // would otherwise spin the loop forever at 120 ms).
    let finished = Arc::new(AtomicU64::new(0));

    // Recover completed chunks from a prior run's sidecar (true resume).
    if let Ok(s) = std::fs::read_to_string(&sidecar) {
        for line in s.lines() {
            if let Ok(i) = line.trim().parse::<u64>() {
                if i < workers as u64 {
                    completed.lock().unwrap().insert(i);
                }
            }
        }
    }
    let recovered = completed.lock().unwrap().len() as u64;
    done.store(recovered * chunk, Ordering::Relaxed);

    // The .part is pre-allocated so parallel chunks can write at any offset;
    // empty regions are fine because only marked chunks are trusted.
    if !part.exists() {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&part)?;
        f.set_len(total)?;
    }
    let t0 = std::time::Instant::now();

    let handles: Vec<_> = (0..workers)
        .filter(|w| !completed.lock().unwrap().contains(&(*w as u64))) // skip recovered
        .map(|w| {
            let start = w as u64 * chunk;
            let end = (start + chunk - 1).min(total - 1);
            let url = url.to_string();
            let part = part.to_path_buf();
            let sidecar = sidecar.to_path_buf();
            let done = Arc::clone(&done);
            let completed = Arc::clone(&completed);
            let finished = Arc::clone(&finished);
            std::thread::spawn(move || -> Result<()> {
                let mut last_err = None;
                for attempt in 0..3 {
                    match fetch_chunk(&url, &part, start, end, &done) {
                        Ok(()) => {
                            // mark chunk done + persist, so a kill resumes here
                            let mut c = completed.lock().unwrap();
                            c.insert(w as u64);
                            let _ = std::fs::write(
                                &sidecar,
                                c.iter()
                                    .map(|i| i.to_string())
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                            );
                            finished.fetch_add(1, Ordering::Relaxed);
                            return Ok(());
                        }
                        Err(e) => last_err = Some(e), // transient drop -> retry
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500 * (attempt + 1)));
                }
                finished.fetch_add(1, Ordering::Relaxed);
                Err(last_err.unwrap_or_else(|| anyhow::anyhow!("chunk {start}-{end} failed")))
            })
        })
        .collect();

    // Animated single-line progress bar. ANSI erase-line + \r redraws ONE line
    // in place (no 100s of scrolled "0 B / …" lines); a "Connecting…" state
    // hides the first-seconds 0-byte stall while the 8 chunk sockets open.
    let w = 30usize;
    let mut last_pct: i64 = -1;
    let mut last_done = 0u64;
    let spawned = handles.len() as u64;
    let mut last_t = std::time::Instant::now();
    let mut connecting = true;
    loop {
        let d = done.load(Ordering::Relaxed);
        let pct = if total > 0 {
            (d as f64 / total as f64 * 100.0) as i64
        } else {
            0
        };
        let dt = last_t.elapsed().as_secs_f64().max(1e-6);
        let rate = (d.saturating_sub(last_done)) as f64 / dt / 1048576.0;
        if d > 0 && connecting {
            connecting = false;
        }
        if connecting {
            // don't spam — one line, redrawn, until the first byte arrives
            print!("\x1b[2K\r  Connecting…  [8× parallel chunks]   ");
        } else if pct != last_pct || rate >= 0.5 {
            let filled = ((pct as f64 / 100.0) * w as f64).round() as usize;
            let bar: String = (0..w).map(|i| if i < filled { '█' } else { '░' }).collect();
            print!(
                "\x1b[2K\r  {bar} {pct:>3}%  {} / {}  [{rate:>6.1} MiB/s]",
                human_bytes(d),
                human_bytes(total)
            );
        }
        let _ = std::io::stdout().flush();
        last_done = d;
        last_t = std::time::Instant::now();
        last_pct = pct;
        // Bail when all bytes arrived OR every worker has returned — a chunk
        // that permanently failed (3 retries) would otherwise spin forever.
        if d >= total || finished.load(Ordering::Relaxed) >= spawned {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(120));
    }
    // The progress loop breaks when done >= total; join the workers and verify
    // EVERY chunk actually completed (sidecar count) before trusting the file.
    for h in handles {
        h.join()
            .map_err(|_| anyhow::anyhow!("download worker panicked"))??;
    }
    let completed_count = completed.lock().unwrap().len();
    if completed_count < workers {
        anyhow::bail!("incomplete download: {completed_count}/{workers} chunks done for {name}");
    }
    print!(
        "\x1b[2K\r  ✓ {name}: {} in {:.1}s\n",
        human_bytes(total),
        t0.elapsed().as_secs_f64()
    );
    let _ = std::io::stdout().flush();

    std::fs::rename(&part, dest)?;
    // `.ok` marker is the ONLY proof a dest is complete (not file length).
    std::fs::write(&ok, "ok")?;
    let _ = std::fs::remove_file(&sidecar);
    Ok(())
}

/// One parallel Range chunk: GET `bytes=start-end`, write at the file offset.
/// `done` is bumped per-read so the progress line moves live (not only when a
/// whole multi-MB chunk completes).
fn fetch_chunk(
    url: &str,
    path: &std::path::Path,
    start: u64,
    end: u64,
    done: &std::sync::atomic::AtomicU64,
) -> Result<()> {
    use std::io::{Read, Seek, Write};
    use std::sync::atomic::Ordering;
    // Per-attempt timeout: ureq's default is no timeout, so a stalled server
    // would block the worker thread forever (combined with the byte-count-only
    // progress loop, the whole install would hang).
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .build(),
    );
    let resp = agent
        .get(url)
        .header("User-Agent", "webrain")
        .header("Range", &format!("bytes={start}-{end}"))
        .call()
        .with_context(|| format!("GET {url} chunk {start}-{end}"))?;
    let mut body = resp.into_body().into_reader();
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    file.seek(std::io::SeekFrom::Start(start))?;
    let mut buf = [0u8; 256 * 1024];
    loop {
        let k = body.read(&mut buf)?;
        if k == 0 {
            break;
        }
        file.write_all(&buf[..k])?;
        done.fetch_add(k as u64, Ordering::Relaxed);
    }
    Ok(())
}

/// Latest llama.cpp CPU release (tag, asset name) per OS/arch — always
/// current, no pinned version to rot. CPU builds are small (~10-18 MB).
fn llama_cpp_release() -> Result<(String, String)> {
    let resp = ureq::get(LLAMA_RELEASES_API)
        .header("User-Agent", "webrain")
        .call()
        .with_context(|| "GET llama.cpp latest release")?;
    let v: Value = resp.into_body().read_json()?;
    let tag = v["tag_name"]
        .as_str()
        .context("llama.cpp tag_name")?
        .to_string();
    let suffix = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "win-cpu-x64.zip",
        ("windows", _) => "win-cpu-arm64.zip",
        ("linux", "aarch64") => "ubuntu-arm64.tar.gz",
        ("linux", _) => "ubuntu-x64.tar.gz",
        ("macos", "aarch64") => "macos-arm64.tar.gz",
        ("macos", _) => "macos-x64.tar.gz",
        _ => anyhow::bail!(
            "llama.cpp has no CPU build for {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    };
    let name = v["assets"]
        .as_array()
        .context("llama.cpp assets")?
        .iter()
        .find_map(|a| {
            a["name"]
                .as_str()
                .filter(|n| n.ends_with(suffix))
                .map(str::to_string)
        })
        .with_context(|| format!("no `{suffix}` asset in llama.cpp {tag}"))?;
    let url = format!("https://github.com/ggml-org/llama.cpp/releases/download/{tag}/{name}");
    Ok((name, url))
}

/// Copy a whole tree into one flat dir — llama.cpp/whisper releases nest their
/// binaries + sibling DLLs under a `bin/` folder; both need to sit together.
fn copy_flat(src: &Path, dest: &Path) -> Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let p = e.path();
        if p.is_dir() {
            copy_flat(&p, dest)?;
        } else if let Some(name) = p.file_name() {
            let _ = std::fs::copy(&p, dest.join(name));
        }
    }
    Ok(())
}

/// Install llama-server (llama.cpp, CPU build) into tools/llama-server/ — the
/// local vision runner, the whisper-cli analog for the vision path.
fn install_llama_server(force: bool) -> Result<PathBuf> {
    let dir = browsers_dir().join("tools").join("llama-server");
    std::fs::create_dir_all(&dir)?;
    let exe = dir.join(bin_name("llama-server"));
    if !force && exe.exists() {
        println!("llama-server already installed: {}", exe.display());
        return Ok(exe);
    }
    let (name, url) = llama_cpp_release()?;
    println!("Downloading llama.cpp ({name})...");
    let tmp = dir.join("_dl");
    download_to_file(&url, &tmp)?;
    let bytes = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    let work = dir.join("_extract");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    extract_archive(&bytes, &work, &name)?;
    copy_flat(&work, &dir)?;
    let _ = std::fs::remove_dir_all(&work);
    if !exe.exists() {
        anyhow::bail!("llama-server not found after extract");
    }
    make_executable(&exe);
    println!("llama-server ready: {}", exe.display());
    Ok(exe)
}

/// Install the Qwen3-VL-2B vision model + mmproj into vision/. Local "hero"
/// vision backend — the whisper GGUF model analog (webrain install vision).
fn install_vision_model(_force: bool) -> Result<(PathBuf, PathBuf)> {
    let dir = browsers_dir().join("vision");
    std::fs::create_dir_all(&dir)?;
    let model = dir.join(VISION_MODEL);
    let mmproj = dir.join(VISION_MMPROJ);
    // No exists() short-circuit here: a truncated file from a dropped download
    // must be re-validated (probe total size) and restarted, not trusted.
    for (file, dest) in [(VISION_MODEL, &model), (VISION_MMPROJ, &mmproj)] {
        let url = format!("{VISION_HF}/{file}");
        println!("[{}/2] {file}", if file == VISION_MODEL { 1 } else { 2 });
        // download_to_file self-validates against the server every run: probes
        // total size, skips only a genuinely-complete file, renames a complete
        // .part, else re-downloads fresh. Never trusts `dest.exists()`.
        download_to_file(&url, dest)?;
    }
    println!("Qwen3-VL-2B ready: {}", dir.display());
    Ok((model, mmproj))
}

/// Install the whole local vision stack (llama-server + Qwen3-VL-2B model +
/// mmproj) — `webrain install vision`. Cache-contained, like the whisper model.
pub fn install_vision(force: bool) -> Result<Value> {
    let server = match install_llama_server(force) {
        Ok(p) => format!("ok: {}", p.display()),
        Err(e) => format!("skip: {e}"),
    };
    let model = match install_vision_model(force) {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("skip: {e}"),
    };
    Ok(json!({"llama-server": server, "qwen3-vl-2b": model}))
}

/// Bundled local vision stack, if installed — `webrain install vision` (the
/// whisper analog): returns (llama-server, Qwen3-VL-2B model, mmproj).
pub fn vision_local() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let server = find_tool("llama-server")?;
    let dir = browsers_dir().join("vision");
    let model = dir.join(VISION_MODEL);
    let mmproj = dir.join(VISION_MMPROJ);
    (model.is_file() && mmproj.is_file()).then_some((server, model, mmproj))
}

pub fn install_watch(force: bool, model: &str) -> Result<Value> {
    let mut out = serde_json::Map::new();
    let ffmpeg = match install_ffmpeg(force) {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("skip: {e}"),
    };
    let ytdlp = match install_ytdlp(force) {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("skip: {e}"),
    };
    let whisper_bin = match install_whisper_bin(force) {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("skip: {e}"),
    };
    let llama = match install_llama_server(force) {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("skip: {e}"),
    };
    let vision = match install_vision_model(force) {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("skip: {e}"),
    };
    let model_res = install_whisper_model(model, force);
    out.insert("ffmpeg".into(), json!(ffmpeg));
    out.insert("yt-dlp".into(), json!(ytdlp));
    out.insert("whisper-cli".into(), json!(whisper_bin));
    out.insert("llama-server".into(), json!(llama));
    out.insert("qwen3-vl-2b".into(), json!(vision));
    out.insert(
        "model".into(),
        json!(match &model_res {
            Ok(p) => format!("ok: {}", p.display()),
            Err(e) => format!("skip: {e}"),
        }),
    );
    // Warnings: what's actually usable after install — one chokepoint covering
    // the whole watch stack (transcript + vision paths).
    let mut warnings: Vec<String> = Vec::new();
    let any_key = |keys: &[&str]| {
        keys.iter()
            .any(|k| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false))
    };
    let stt_keys = ["GROQ_API_KEY", "OPENAI_API_KEY", "FIREWORKS_API_KEY"];
    let vision_keys = ["GROQ_API_KEY", "OPENAI_API_KEY"];
    let model_file = browsers_dir()
        .join("whisper")
        .join(format!("ggml-{model}.bin"));
    if !(find_tool("whisper-cli").is_some() && model_file.exists()) && !any_key(&stt_keys) {
        warnings.push(
            "no local whisper (binary + model) and no cloud STT key — videos without captions will return an empty transcript; set GROQ_API_KEY | OPENAI_API_KEY | FIREWORKS_API_KEY or run `webrain install whisper --model small.en`"
                .to_string(),
        );
    }
    let local_vision = find_tool("llama-server").is_some()
        && browsers_dir().join("vision").join(VISION_MODEL).exists();
    if !any_key(&vision_keys) && !local_vision {
        warnings.push(
            "no vision key and no local Qwen3-VL-2B — `watch --vision` will return vision_error; set GROQ_API_KEY | OPENAI_API_KEY or run `webrain install vision` (bundles llama-server + Qwen3-VL-2B locally, like whisper)"
                .to_string(),
        );
    }
    out.insert("warnings".into(), json!(warnings));
    Ok(Value::Object(out))
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
