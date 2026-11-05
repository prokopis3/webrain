//! The chat page, as files the binary does not carry.
//!
//! `webrain chat --serve` serves a page that can approve a shell command, so the
//! bytes must be the bytes this build shipped. Embedding them in the binary was
//! the easy way to guarantee that; this module keeps them out of the binary
//! (285 KB of JS/CSS/HTML in every platform download) and buys the same
//! guarantee from digests `build.rs` computes from the files the build used.
//!
//! Every candidate directory is verified before a single byte is served, and a
//! mismatch is refused with the file name and both digests — a tampered
//! `chat.js` is exactly the attack that matters here, since the page is what
//! asks the operator to allow a tool call.
//!
//! Where the files may be, best first:
//!   1. `WEBRAIN_CHAT_UI` — an explicit directory
//!   2. the engine cache (`webrain install chat-ui` fills it)
//!   3. `webrain-chat-ui/` next to the binary (portable zip layouts)
//!   4. `webrain-cli/assets/` in a checkout, so `cargo run` needs no install

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

// `build.rs` writes this: `[(name, sha256 hex, bytes); 4]` — the digests of the
// four files this build's `assets/` held.
include!(concat!(env!("OUT_DIR"), "/chat_ui_manifest.rs"));

/// The four files, verified and ready to serve.
pub struct Ui {
    pub html: String,
    pub css: String,
    pub js: String,
    pub markdown: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The four files of `dir`, or the reason they are not this build's.
fn read_verified(dir: &Path) -> Result<[String; 4], String> {
    let mut files: [String; 4] = Default::default();
    for (i, (name, sha, len)) in MANIFEST.iter().enumerate() {
        let path = dir.join(name);
        let bytes = std::fs::read(&path).map_err(|e| format!("{name}: {e}"))?;
        if bytes.len() as u64 != *len {
            return Err(format!(
                "{name}: {} bytes, expected {len}",
                bytes.len()
            ));
        }
        let got = sha256_hex(&bytes);
        if !got.eq_ignore_ascii_case(sha) {
            return Err(format!("{name}: sha256 {got}, expected {sha}"));
        }
        files[i] = String::from_utf8(bytes).map_err(|_| format!("{name}: not UTF-8"))?;
    }
    Ok(files)
}

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("WEBRAIN_CHAT_UI") {
        out.push(PathBuf::from(p));
    }
    out.push(webrain_core::install::chat_ui_dir());
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.join("webrain-chat-ui")))
    {
        out.push(dir);
    }
    // A checkout: `cargo run -- chat --serve` with no install step. Verified like
    // every other candidate, so this is convenience, not a hole.
    out.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets")));
    out
}

/// Load the UI, or say exactly what is wrong and how to fix it.
pub fn load() -> Result<Ui, String> {
    let mut tried: Vec<String> = Vec::new();
    for dir in candidates() {
        if !dir.is_dir() {
            continue;
        }
        match read_verified(&dir) {
            Ok([html, css, js, markdown]) => return Ok(Ui { html, css, js, markdown }),
            Err(e) => tried.push(format!("{}: {e}", dir.display())),
        }
    }
    let detail = if tried.is_empty() {
        "no chat UI directory exists on this machine".to_string()
    } else {
        tried.join("\n  ")
    };
    Err(format!(
        "the chat UI is missing or is not this build's:\n  {detail}\n  \
         fix: `webrain install chat-ui`, or set WEBRAIN_CHAT_UI to a directory \
         holding the four files"
    ))
}

/// `webrain install chat-ui [--from <url|file>]` — unpack the archive into the
/// engine cache and prove it matches before reporting success.
///
/// The default URL is the release of *this* version, not `latest`: an archive
/// from a newer build would fail the digest check, so there is nothing to gain
/// from guessing.
pub fn install(force: bool, from: Option<&str>) -> anyhow::Result<PathBuf> {
    let dir = webrain_core::install::chat_ui_dir();
    if !force && read_verified(&dir).is_ok() {
        println!("chat UI already installed: {}", dir.display());
        return Ok(dir);
    }
    let fallback_url;
    let src = match from {
        Some(f) => f,
        None => {
            fallback_url = format!(
                "https://github.com/prokopis3/webrain/releases/download/v{}/webrain-chat-ui.tar.gz",
                env!("CARGO_PKG_VERSION")
            );
            &fallback_url
        }
    };
    println!("chat UI: {src}");
    let bytes = if src.starts_with("http") {
        webrain_core::install::download_bytes(src)?
    } else {
        std::fs::read(src).map_err(|e| anyhow::anyhow!("{src}: {e}"))?
    };
    // Unpack beside the target and verify there, then swap: a bad archive must
    // leave the working install alone instead of replacing it with nothing.
    let staging = dir.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let checked = webrain_core::install::extract_archive(&bytes, &staging, "webrain-chat-ui.tar.gz")
        .and_then(|()| {
            read_verified(&staging)
                .map_err(|e| anyhow::anyhow!("the unpacked UI is not this build's: {e}"))
        });
    if let Err(e) = checked {
        let _ = std::fs::remove_dir_all(&staging);
        anyhow::bail!("{e} — nothing was installed from it");
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::rename(&staging, &dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"))
    }

    #[test]
    fn the_builds_own_assets_verify() {
        let files = read_verified(&assets()).expect("the committed assets match the manifest");
        assert!(files[2].contains("<"), "chat.js came back as text");
        assert!(files[0].contains("</html>"), "chat.html came back as text");
    }

    #[test]
    fn one_changed_byte_is_refused() {
        // The security gate: a page that can approve a shell command must not be
        // served from a file somebody edited.
        let dir = std::env::temp_dir().join(format!("webrain-chat-ui-tamper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, _, _) in MANIFEST.iter() {
            std::fs::copy(assets().join(name), dir.join(name)).unwrap();
        }
        // Same length, different bytes: only the digest can catch this.
        let mut js = std::fs::read(dir.join("chat.js")).unwrap();
        js[0] = b'!';
        std::fs::write(dir.join("chat.js"), &js).unwrap();
        let err = read_verified(&dir).expect_err("a tampered file must not verify");
        assert!(err.starts_with("chat.js: sha256"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
