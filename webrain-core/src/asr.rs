//! Local speech-to-text for dictation: NVIDIA Nemotron 3.5 ASR
//! (`nemotron-3.5-asr-streaming-0.6b`, 40 language-locales, prompt-conditioned)
//! through the `nemo-speech` CLI from NeMo-Speech.cpp.
//!
//! Why a shell-out and not a dependency: that runtime is a native GGML binary
//! with a prebuilt archive per platform, so `webrain install nemotron` unpacks
//! it next to the model and this module stays the only glue — no ONNX Runtime,
//! no Python, no weights linked into this binary. Same shape as `whisper-cli`
//! in `video.rs`, which is the point: two recognizers, one convention.
//!
//! Everything here is offline. The audio a user dictates never leaves the
//! machine, and nothing is kept after the text comes back.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

/// `WEBRAIN_ASR_BIN`, else the bundled `tools/nemo-speech/` that
/// `webrain install nemotron` unpacks, else PATH.
pub fn recognizer() -> Option<PathBuf> {
    std::env::var_os("WEBRAIN_ASR_BIN")
        .map(PathBuf::from)
        .or_else(|| crate::install::find_tool("nemo-speech"))
}

/// `WEBRAIN_ASR_MODEL`, else the largest `*.gguf` under `<cache>/nemotron/` —
/// several quants may coexist and the bigger file is the more accurate one.
pub fn model_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WEBRAIN_ASR_MODEL") {
        return Some(PathBuf::from(p));
    }
    let dir = crate::install::browsers_dir().join("nemotron");
    let mut found: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("gguf"))
        })
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
        .collect();
    found.sort_by_key(|(len, _)| std::cmp::Reverse(*len));
    found.into_iter().next().map(|(_, p)| p)
}

/// What is missing, in one sentence a user can act on — `None` when dictation
/// is ready. The page shows this instead of a record button that fails.
pub fn unavailable() -> Option<String> {
    if recognizer().is_none() {
        return Some(
            "no nemo-speech — run `webrain install nemotron` (or set WEBRAIN_ASR_BIN)".into(),
        );
    }
    match model_path() {
        Some(p) if p.is_file() => None,
        _ => Some(
            "no Nemotron GGUF — run `webrain install nemotron` (or set WEBRAIN_ASR_MODEL)".into(),
        ),
    }
}

/// One 16 kHz mono WAV → text. `lang` is a locale the model knows (`en-US`,
/// `de-DE`, …) or `auto` to let it detect and tag the language itself.
pub fn transcribe(wav: &Path, lang: &str) -> Result<String> {
    let bin =
        recognizer().ok_or_else(|| anyhow!("no nemo-speech — run `webrain install nemotron`"))?;
    let model =
        model_path().ok_or_else(|| anyhow!("no Nemotron GGUF — run `webrain install nemotron`"))?;
    if !model.is_file() {
        return Err(anyhow!("model not found at {}", model.display()));
    }
    let mut cmd = std::process::Command::new(&bin);
    // Offline decoding (the model's [56, 13] preset) is what a dictation take
    // wants: full context, best accuracy. `--live` exists for streaming
    // captions and is not this.
    cmd.arg("transcribe").arg(wav).arg("--model").arg(&model);
    // `--quiet` keeps progress off stdout so what comes back is only the
    // transcript; `-f text` is the default and is stated anyway, because a
    // default that changes would otherwise silently return JSON.
    cmd.args(["--quiet", "-f", "text"]);
    let lang = lang.trim();
    if !lang.is_empty() {
        cmd.arg("--language").arg(lang);
    }
    let out = cmd
        .output()
        .with_context(|| format!("could not run {}", bin.display()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!(
            "nemo-speech failed: {}",
            err.lines().next().unwrap_or("no stderr")
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A recording in whatever the browser produced (webm, ogg, m4a, wav) → text.
/// The recognizer wants 16 kHz mono WAV, so `ffmpeg` does that first; both the
/// converted file and the original are the caller's to delete.
pub fn transcribe_audio(src: &Path, lang: &str) -> Result<String> {
    let dir = src
        .parent()
        .ok_or_else(|| anyhow!("audio file has no parent dir"))?;
    let wav = crate::video::extract_audio(&src.to_string_lossy(), dir)?;
    let text = transcribe(Path::new(&wav), lang)?;
    let _ = std::fs::remove_file(&wav);
    if text.is_empty() {
        return Err(anyhow!("the recognizer returned no text"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing model is reported, never guessed at — the chat route turns this
    /// string into the install hint the page shows.
    #[test]
    fn unavailable_names_what_is_missing() {
        // Only meaningful where nothing is installed; when a machine has the
        // model the function must say "ready" instead of naming a file.
        match unavailable() {
            None => assert!(model_path().is_some_and(|p| p.is_file())),
            Some(msg) => assert!(
                msg.contains("webrain install nemotron"),
                "unhelpful hint: {msg}"
            ),
        }
    }
}
