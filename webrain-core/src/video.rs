//! Watch any video (URL or local file): timestamped transcript + frames, so
//! the LLM can "see and hear" it and summarize.
//!
//! Pipeline (borrow: bradautomates/claude-video `/watch`, MIT — same steps, in
//! Rust, zero new deps): yt-dlp captions → (fallback) Whisper STT → ffmpeg
//! frames. Everything that does real work shells out to binaries already on
//! PATH (yt-dlp / ffprobe / ffmpeg); the only network call we make is the
//! Whisper REST upload via the pooled ureq agent.
//!
//! Secrets: STT keys come from env only (`GROQ_API_KEY` → `OPENAI_API_KEY` →
//! `FIREWORKS_API_KEY`; model override `WEBRAIN_STT_MODEL`) and are never
//! returned or logged.

use crate::engines::browser_agent;
use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// Transcript only — no frames, and (when captions exist) no video download.
    Transcript,
    /// Fast keyframe pass (`-skip_frame nokey`), cheap, capped.
    Efficient,
    /// Scene-aware frames (default) — best summarization signal.
    Balanced,
}

impl Detail {
    pub fn parse(s: &str) -> Detail {
        match s {
            "transcript" => Detail::Transcript,
            "efficient" => Detail::Efficient,
            _ => Detail::Balanced,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Detail::Transcript => "transcript",
            Detail::Efficient => "efficient",
            Detail::Balanced => "balanced",
        }
    }
    /// Max frames for this detail at any duration.
    fn hard_cap(self) -> usize {
        match self {
            Detail::Transcript => 0,
            Detail::Efficient => 50,
            Detail::Balanced => 100,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SttBackend {
    Whisper,
    Gemini,
}

impl SttBackend {
    pub fn parse(s: &str) -> SttBackend {
        match s {
            "gemini" => SttBackend::Gemini,
            _ => SttBackend::Whisper,
        }
    }
}

#[derive(Clone)]
pub struct WatchOpts {
    pub detail: Detail,
    pub max_frames: Option<usize>,
    pub resolution: u32,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub out_dir: Option<String>,
    pub no_whisper: bool,
    pub stt_backend: SttBackend,
    pub vision: bool,
}

impl Default for WatchOpts {
    fn default() -> Self {
        WatchOpts {
            detail: Detail::Balanced,
            max_frames: None,
            resolution: 512,
            start: None,
            end: None,
            out_dir: None,
            no_whisper: false,
            stt_backend: SttBackend::Whisper,
            vision: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Default, Clone)]
pub struct Probe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub index: usize,
    pub t: f64,
    pub path: String,
}

// ---------------------------------------------------------------------------
// Subprocess helper
// ---------------------------------------------------------------------------

fn run(cmd: &mut std::process::Command) -> Result<std::process::Output> {
    cmd.output()
        .map_err(|e| anyhow!("failed to run {}: {e}", cmd.get_program().to_string_lossy()))
}

// ---------------------------------------------------------------------------
// WebVTT → segments
// ---------------------------------------------------------------------------

static VTT_TS_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
fn vtt_ts_re() -> &'static regex::Regex {
    VTT_TS_RE.get_or_init(|| {
        // Cue header, `MM:SS.mmm` or `HH:MM:SS.mmm` (optional hour group), no
        // look-around (regex crate). Text body comes from the rest of the block.
        regex::Regex::new(
            r"^(?:(\d{1,2}):)?(\d{1,2}):(\d{2})[.:](\d{3})\s+-->\s+(?:(\d{1,2}):)?(\d{1,2}):(\d{2})[.:](\d{3})",
        )
        .expect("static vtt timestamp regex")
    })
}

/// Parse WebVTT into timestamped segments. Accepts `MM:SS.mmm` and
/// `HH:MM:SS.mmm` cue times, keeps multi-line cue text, skips empty cues and
/// `NOTE`/`WEBVTT` header blocks.
pub fn vtt_segments(vtt: &str) -> Vec<Segment> {
    let ts_re = vtt_ts_re();
    let mut out = Vec::new();
    for block in vtt.replace("\r\n", "\n").split("\n\n") {
        let mut lines = block.lines();
        let Some(header) = lines.next() else { continue };
        let Some(cap) = ts_re.captures(header) else {
            continue;
        };
        let ts = |h: Option<&str>, m: &str, s: &str, ms: &str| -> f64 {
            let h: f64 = h.and_then(|v| v.parse().ok()).unwrap_or(0.0);
            let m: f64 = m.parse().unwrap_or(0.0);
            let s: f64 = s.parse().unwrap_or(0.0);
            let ms: f64 = ms.parse().unwrap_or(0.0);
            h * 3600.0 + m * 60.0 + s + ms / 1000.0
        };
        let start = ts(cap.get(1).map(|m| m.as_str()), &cap[2], &cap[3], &cap[4]);
        let end = ts(cap.get(5).map(|m| m.as_str()), &cap[6], &cap[7], &cap[8]);
        let text = lines.collect::<Vec<_>>().join("\n").trim().to_string();
        if !text.is_empty() {
            out.push(Segment { start, end, text });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// ffprobe
// ---------------------------------------------------------------------------

/// ffprobe durations come back as strings (`"9.666667"`); accept number or
/// string and fall back to 0.0 so a missing field never panics.
fn jnum(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        .unwrap_or(0.0)
}

pub fn ffprobe(path: &str) -> Result<Probe> {
    let out = run(std::process::Command::new(tool_path("ffprobe"))
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path))?;
    let v: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| anyhow!("ffprobe JSON parse failed: {e}"))?;
    let mut p = Probe {
        duration: jnum(v["format"].get("duration").unwrap_or(&Value::Null)),
        ..Default::default()
    };
    // Fragmented MP4 (reels/DASH) sometimes omits format.duration — fall back
    // to a stream's duration so frame budgeting still works.
    if p.duration <= 0.0 {
        for s in v["streams"].as_array().cloned().unwrap_or_default() {
            let d = jnum(s.get("duration").unwrap_or(&Value::Null));
            if d > 0.0 {
                p.duration = d;
                break;
            }
        }
    }
    for s in v["streams"].as_array().cloned().unwrap_or_default() {
        match s.get("codec_type").and_then(|c| c.as_str()).unwrap_or("") {
            "video" => {
                p.width = s.get("width").and_then(|w| w.as_u64()).unwrap_or(0) as u32;
                p.height = s.get("height").and_then(|h| h.as_u64()).unwrap_or(0) as u32;
            }
            "audio" => p.has_audio = true,
            _ => {}
        }
    }
    Ok(p)
}

// ---------------------------------------------------------------------------
// yt-dlp
// ---------------------------------------------------------------------------

/// Download the video (+ English captions) into `dir` as `video.<ext>`.
/// Returns `(video_path, optional_vtt_path)`.
fn ytdlp_download(url: &str, dir: &Path) -> Result<(String, Option<String>)> {
    let _ = std::fs::create_dir_all(dir);
    // 720p ceiling: plenty for frame reading, much faster than 4K.
    let out = run(std::process::Command::new(tool_path("yt-dlp"))
        .arg("-N")
        .arg("8")
        .arg("-f")
        .arg("bv*[height<=720]+ba/b[height<=720]/b")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--write-subs")
        .arg("--write-auto-subs")
        .arg("--sub-langs")
        .arg("en.*")
        .arg("--sub-format")
        .arg("vtt")
        .arg("--convert-subs")
        .arg("vtt")
        .arg("--no-playlist")
        .arg("--ignore-errors")
        .arg("-o")
        .arg(dir.join("video.%(ext)s"))
        .arg("--")
        .arg(url))?;
    // yt-dlp can exit non-zero on a subtitle 429 while the video still landed.
    let video = find_video(dir);
    match video {
        Some(v) => Ok((v, find_vtt(dir))),
        None => Err(anyhow!(
            "yt-dlp produced no video in {}:\n{}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        )),
    }
}

/// Captions only (`--skip-download`) — the cheap path for transcript detail.
fn ytdlp_captions(url: &str, dir: &Path) -> Option<String> {
    let _ = std::fs::create_dir_all(dir);
    let _ = run(std::process::Command::new(tool_path("yt-dlp"))
        .arg("--skip-download")
        .arg("--write-info-json")
        .arg("--write-subs")
        .arg("--write-auto-subs")
        .arg("--sub-langs")
        .arg("en.*")
        .arg("--sub-format")
        .arg("vtt")
        .arg("--convert-subs")
        .arg("vtt")
        .arg("--no-playlist")
        .arg("--ignore-errors")
        .arg("-o")
        .arg(dir.join("video.%(ext)s"))
        .arg("--")
        .arg(url));
    find_vtt(dir)
}

fn find_video(dir: &Path) -> Option<String> {
    for ext in ["mp4", "mkv", "webm", "mov", "m4v"] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with("video") && n.ends_with(ext) {
                    return Some(e.path().to_string_lossy().to_string());
                }
            }
        }
    }
    None
}

/// Prefer an English VTT; fall back to any `video.*.vtt`.
fn find_vtt(dir: &Path) -> Option<String> {
    let mut all: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            (n.starts_with("video") && n.ends_with(".vtt")).then_some(n)
        })
        .collect();
    all.sort();
    all.iter()
        .find(|n| n.contains(".en."))
        .or_else(|| all.first())
        .map(|n| dir.join(n).to_string_lossy().to_string())
}

fn info_title(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("video.info.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("title").and_then(|t| t.as_str()).map(String::from))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// ffmpeg frames
// ---------------------------------------------------------------------------

fn frame_budget(duration: f64, detail: Detail) -> usize {
    match detail {
        Detail::Transcript => 0,
        Detail::Efficient => 50,
        Detail::Balanced => {
            if duration <= 30.0 {
                20
            } else if duration <= 60.0 {
                40
            } else if duration <= 180.0 {
                60
            } else if duration <= 600.0 {
                80
            } else {
                100
            }
        }
    }
}

fn frames(path: &str, dir: &Path, probe: &Probe, opts: &WatchOpts) -> Result<Vec<Frame>> {
    let resolution = opts.resolution.max(64);
    let cap = opts
        .max_frames
        .unwrap_or_else(|| frame_budget(probe.duration, opts.detail))
        .clamp(1, opts.detail.hard_cap().max(1));
    let outdir = dir.join("frames");
    let _ = std::fs::create_dir_all(&outdir);
    clear_jpgs(&outdir);
    let scale = format!("scale={resolution}:-1");
    let pattern = outdir.join("frame_%04d.jpg");

    let mut cmd = std::process::Command::new(tool_path("ffmpeg"));
    cmd.args(["-hide_banner", "-loglevel", "info", "-y"]);
    if let Some(s) = opts.start {
        cmd.arg("-ss").arg(format!("{s:.3}"));
    }
    if let Some(e) = opts.end {
        cmd.arg("-to").arg(format!("{e:.3}"));
    }
    match opts.detail {
        Detail::Efficient => {
            // Keyframes only: ~1/s, no scene detection cost.
            cmd.args(["-skip_frame", "nokey", "-i", path]);
            cmd.arg("-vf").arg(format!("{scale},showinfo"));
        }
        _ => {
            // Scene-aware: first frame + any scene cut > 0.2, showinfo for pts.
            cmd.arg("-i").arg(path);
            cmd.arg("-vf").arg(format!(
                "select='eq(n\\,0)+gt(scene\\,0.2)',{scale},showinfo"
            ));
        }
    }
    cmd.arg("-vsync").arg("vfr");
    cmd.arg("-frames:v").arg(cap.to_string());
    cmd.arg("-q:v").arg("4");
    cmd.arg(&pattern);
    let out = run(&mut cmd)?;

    let mut extracted = parse_frame_paths(&outdir, &out.stderr);
    // Uniform-fps fallback when scene detection yields too few frames (reels
    // are usually one continuous shot — no hard cuts — so scene select alone
    // would give a single frame). Needs a usable duration to budget the fps.
    if extracted.len() < 4 && opts.detail != Detail::Efficient && probe.duration > 0.0 {
        // Uniform-fps fallback (e.g. no scene cuts, or a weird filter pass).
        clear_jpgs(&outdir);
        let fps = (cap as f64 / probe.duration.max(1.0)).min(2.0);
        let out = run(std::process::Command::new(tool_path("ffmpeg"))
            .args(["-hide_banner", "-loglevel", "info", "-y"])
            .arg("-i")
            .arg(path)
            .arg("-vf")
            .arg(format!("fps={fps:.4},{scale},showinfo"))
            .arg("-frames:v")
            .arg(cap.to_string())
            .arg("-q:v")
            .arg("4")
            .arg(&pattern))?;
        extracted = parse_frame_paths(&outdir, &out.stderr);
    }
    if extracted.len() > cap {
        extracted.truncate(cap);
    }
    Ok(extracted)
}

fn clear_jpgs(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("jpg") {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

/// Pair written `frame_NNNN.jpg` files with their `pts_time:` from ffmpeg stderr
/// (showinfo emits one per written frame, in order).
fn parse_frame_paths(dir: &Path, stderr: &[u8]) -> Vec<Frame> {
    let stderr = String::from_utf8_lossy(stderr);
    let ts: Vec<f64> = regex::Regex::new(r"pts_time:([0-9.]+)")
        .unwrap()
        .captures_iter(&stderr)
        .filter_map(|c| c[1].parse().ok())
        .collect();
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    (n.starts_with("frame_") && n.ends_with(".jpg")).then_some(n)
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
        .into_iter()
        .enumerate()
        .map(|(i, n)| Frame {
            index: i,
            t: ts.get(i).copied().unwrap_or(0.0),
            path: dir.join(n).to_string_lossy().to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Audio → Whisper
// ---------------------------------------------------------------------------

fn extract_audio(video: &str, dir: &Path) -> Result<String> {
    // 16kHz mono s16le WAV: the format whisper-cli (whisper.cpp) requires, and
    // the cloud Whisper APIs accept too — one audio file, both backends.
    let out_path = dir.join("audio.wav");
    let out = run(std::process::Command::new(tool_path("ffmpeg"))
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .arg("-i")
        .arg(video)
        .args(["-vn", "-acodec", "pcm_s16le", "-ar", "16000", "-ac", "1"])
        .arg(&out_path))?;
    let ok = std::fs::metadata(&out_path)
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    if !ok {
        return Err(anyhow!(
            "ffmpeg produced no audio (video may have no audio track): {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out_path.to_string_lossy().to_string())
}

/// Resolve the Whisper-compatible provider: first key that's set wins
/// (Groq → OpenAI → Fireworks); `WEBRAIN_STT_MODEL` overrides the per-provider
/// default model. Pure so it's unit-testable without touching the env.
/// ponytail: all three speak the OpenAI multipart transcriptions shape — a new
/// provider is one row in this list, no new code. Gemini is NOT here: its
/// audio API is inline-file JSON, not multipart (see SttBackend::Gemini stub).
fn stt_providers(
    groq: Option<&str>,
    openai: Option<&str>,
    fireworks: Option<&str>,
    model_override: Option<&str>,
) -> Vec<(String, String, String)> {
    let candidates: [(&str, &str, &str, Option<&str>); 3] = [
        (
            "GROQ_API_KEY",
            "https://api.groq.com/openai/v1/audio/transcriptions",
            "whisper-large-v3",
            groq,
        ),
        (
            "OPENAI_API_KEY",
            "https://api.openai.com/v1/audio/transcriptions",
            "whisper-1",
            openai,
        ),
        (
            "FIREWORKS_API_KEY",
            "https://api.fireworks.ai/inference/v1/audio/transcriptions",
            "whisper-v3",
            fireworks,
        ),
    ];
    candidates
        .into_iter()
        .filter_map(|(_name, ep, def, key)| {
            key.map(|k| {
                (
                    ep.to_string(),
                    model_override.unwrap_or(def).to_string(),
                    k.to_string(),
                )
            })
        })
        .collect()
}

#[cfg(test)]
/// First configured STT provider (Groq → OpenAI → Fireworks); the runtime path
/// uses `stt_providers` (all of them) so it can fall back on a 429/5xx.
fn stt_provider(
    groq: Option<&str>,
    openai: Option<&str>,
    fireworks: Option<&str>,
    model_override: Option<&str>,
) -> Option<(String, String, String)> {
    stt_providers(groq, openai, fireworks, model_override)
        .into_iter()
        .next()
}

fn whisper_transcribe(audio: &str, backend: SttBackend) -> Result<Vec<Segment>> {
    let bytes = std::fs::read(audio)?;
    if bytes.len() > 24 * 1024 * 1024 {
        return Err(anyhow!("audio >24MB exceeds the Whisper upload limit"));
    }
    let providers: Vec<(String, String, String)> = match backend {
        // ponytail: Gemini has a different (inline-file JSON) API shape — add a
        // real arm when a second audio API shape is actually needed.
        SttBackend::Gemini => {
            return Err(anyhow!(
                "gemini STT backend is a stub — set WEBRAIN_STT_BACKEND=whisper (Groq/OpenAI/Fireworks) or add the gemini arm"
            ));
        }
        SttBackend::Whisper => {
            let groq = std::env::var("GROQ_API_KEY").ok();
            let openai = std::env::var("OPENAI_API_KEY").ok();
            let fireworks = std::env::var("FIREWORKS_API_KEY").ok();
            let model_override = std::env::var("WEBRAIN_STT_MODEL").ok();
            stt_providers(
                groq.as_deref(),
                openai.as_deref(),
                fireworks.as_deref(),
                model_override.as_deref(),
            )
        }
    };
    if providers.is_empty() {
        return Err(anyhow!(
            "no STT key — set GROQ_API_KEY, OPENAI_API_KEY, or FIREWORKS_API_KEY (model: WEBRAIN_STT_MODEL)"
        ));
    }
    // ponytail: one provider can 429/5xx — try each configured provider in order
    // (Groq → OpenAI → Fireworks) instead of failing the whole watch, mirroring
    // the cloud→local vision fallback above.
    let mut last_err: Option<anyhow::Error> = None;
    for (endpoint, model, key) in providers {
        let boundary = format!(
            "----webrain{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let mut body = Vec::new();
        for (name, value) in [
            ("model", model.as_str()),
            ("response_format", "verbose_json"),
        ] {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.mp3\"\r\nContent-Type: audio/mpeg\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(&bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let resp = match browser_agent()
            .post(endpoint)
            .header("Authorization", format!("Bearer {key}"))
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .send(body)
        {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(e.into());
                continue;
            }
        };
        let v: Value = match resp.into_body().read_json() {
            Ok(v) => v,
            Err(e) => {
                last_err = Some(e.into());
                continue;
            }
        };
        let segments = v["segments"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| {
                let text = s
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                (!text.is_empty()).then(|| Segment {
                    start: s.get("start").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    end: s.get("end").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    text,
                })
            })
            .collect();
        return Ok(segments);
    }
    Err(last_err.unwrap_or_else(|| anyhow!("all STT providers failed")))
}

// ---------------------------------------------------------------------------
// Audio → Local whisper-cli (whisper.cpp)
// ---------------------------------------------------------------------------

/// Resolve a tool binary: bundled mono package (`<browsers_dir>/tools/<tool>/`)
/// else PATH. Keeps `webrain watch` self-contained after `webrain install watch`.
fn tool_path(name: &str) -> String {
    crate::install::find_tool(name)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Default local model path (what `webrain install whisper` downloads).
fn default_whisper_model() -> String {
    crate::install::browsers_dir()
        .join("whisper")
        .join("ggml-small.en.bin")
        .to_string_lossy()
        .to_string()
}

/// Transcribe via whisper.cpp's `whisper-cli` — fully local/offline/private,
/// free, CPU or GPU. Binary: PATH or `WEBRAIN_WHISPER_BIN`; model:
/// `WEBRAIN_WHISPER_MODEL` or the `webrain install whisper` default.
/// ponytail: no `--language`/threads tuning — whisper-cli defaults are fine for
/// segment-level summarize transcripts; tune only if accuracy measurably hurts.
fn whisper_local(audio: &str, dir: &Path) -> Result<Vec<Segment>> {
    let bin = std::env::var("WEBRAIN_WHISPER_BIN")
        .ok()
        .or_else(|| {
            crate::install::find_tool("whisper-cli")
                .map(|p| p.to_string_lossy().to_string())
        })
        .ok_or_else(|| {
            anyhow!(
                "whisper-cli not found on PATH — install whisper.cpp (e.g. `scoop install whisper.cpp`) or set WEBRAIN_WHISPER_BIN"
            )
        })?;
    let model = std::env::var("WEBRAIN_WHISPER_MODEL")
        .ok()
        .unwrap_or_else(default_whisper_model);
    if !Path::new(&model).exists() {
        return Err(anyhow!(
            "whisper model not found at {model} — run `webrain install whisper` to download it (or set WEBRAIN_WHISPER_MODEL)"
        ));
    }
    let base = dir.join("local");
    let out = run(std::process::Command::new(&bin)
        .arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(audio)
        .arg("-np") // no progress prints
        .arg("-oj") // output json
        .arg("-of")
        .arg(&base))?; // writes <base>.json
    if !out.status.success() {
        return Err(anyhow!(
            "whisper-cli failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let json_path = format!("{}.json", base.to_string_lossy());
    let text = std::fs::read_to_string(&json_path)?;
    let v: Value = serde_json::from_str(&text)?;
    let segs = whisper_json_segments(&v);
    if segs.is_empty() {
        return Err(anyhow!("whisper-cli produced no segments"));
    }
    Ok(segs)
}

/// Parse whisper-cli `--output-json`:
/// `{"transcription": [{"timestamps":{"from":"..","to":".."}, "offsets":{"from":ms,"to":ms}, "text":".."}]}`
/// Prefers numeric `offsets` (ms), falls back to `timestamps` (`HH:MM:SS,mmm`).
fn whisper_json_segments(v: &Value) -> Vec<Segment> {
    fn parse_ts(s: &str) -> f64 {
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        let sec = |p: &str| p.replace(',', ".").parse::<f64>().unwrap_or(0.0);
        match parts.len() {
            3 => sec(parts[0]) * 3600.0 + sec(parts[1]) * 60.0 + sec(parts[2]),
            2 => sec(parts[0]) * 60.0 + sec(parts[1]),
            _ => sec(s),
        }
    }
    fn seg_time(s: &Value, off: &str, ts: &str) -> f64 {
        s.get("offsets")
            .and_then(|o| o.get(off))
            .and_then(|x| x.as_f64())
            .map(|ms| ms / 1000.0)
            .or_else(|| {
                s.get("timestamps")
                    .and_then(|t| t.get(ts))
                    .and_then(|x| x.as_str())
                    .map(parse_ts)
            })
            .unwrap_or(0.0)
    }
    v["transcription"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let text = s
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            (!text.is_empty()).then(|| Segment {
                start: seg_time(&s, "from", "from"),
                end: seg_time(&s, "to", "to"),
                text,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Frame vision — text captions so a client that can't render images (or a
// text-only model) still "sees" the video. ponytail: provider chain is Groq
// qwen3.6-27b (vision-capable; llama-3.2-vision was decommissioned) → OpenAI
// gpt-4o-mini → LOCAL Qwen3-VL-2B (bundled llama-server, the "hero" when no
// cloud key is set — `webrain install vision`, whisper-style). DeepSeek's
// hosted API is TEXT-ONLY — live-verified 2026-08-06 (rejects image_url
// content), so it's intentionally absent.
// ---------------------------------------------------------------------------

enum VisionTarget {
    Cloud {
        endpoint: String,
        model: String,
        key: String,
    },
    Local {
        model: PathBuf,
        mmproj: PathBuf,
    },
}

fn vision_target(
    groq: Option<&str>,
    openai: Option<&str>,
    model: Option<&Path>,
    mmproj: Option<&Path>,
) -> Option<VisionTarget> {
    const GROQ: (&str, &str) = (
        "https://api.groq.com/openai/v1/chat/completions",
        "qwen/qwen3.6-27b",
    );
    const OPENAI: (&str, &str) = ("https://api.openai.com/v1/chat/completions", "gpt-4o-mini");
    if let Some(k) = groq {
        Some(VisionTarget::Cloud {
            endpoint: GROQ.0.into(),
            model: GROQ.1.into(),
            key: k.into(),
        })
    } else if let Some(k) = openai {
        Some(VisionTarget::Cloud {
            endpoint: OPENAI.0.into(),
            model: OPENAI.1.into(),
            key: k.into(),
        })
    } else if let (Some(m), Some(p)) = (model, mmproj) {
        Some(VisionTarget::Local {
            model: m.to_path_buf(),
            mmproj: p.to_path_buf(),
        })
    } else {
        None
    }
}

/// OpenAI-compatible chat/completions POST, surfacing the API's own error
/// body on non-2xx (status-as-error off — a bare "http status: 400" hid a
/// decommissioned-model cause before). `auth: None` = local server (no key).
pub(crate) fn post_vision(
    agent: &ureq::Agent,
    endpoint: &str,
    auth: Option<&str>,
    body: &Value,
) -> Result<String> {
    let mut req = agent
        .post(endpoint)
        .header("Content-Type", "application/json");
    if let Some(k) = auth {
        req = req.header("Authorization", format!("Bearer {k}"));
    }
    let resp = req.send(serde_json::to_vec(body)?)?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let detail = resp.into_body().read_to_string().unwrap_or_default();
        return Err(anyhow!("vision API http {status}: {detail}"));
    }
    let v: Value = resp.into_body().read_json()?;
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("vision model returned no content"))
}

/// Spawn the bundled llama-server with Qwen3-VL-2B (model + mmproj) on a free
/// port, wait for /health, return (child, OpenAI-compat endpoint). Caller kills.
pub(crate) fn spawn_llama_server(
    exe: &Path,
    model: &Path,
    mmproj: &Path,
) -> Result<(std::process::Child, String)> {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr().map(|a| a.port()))
        .unwrap_or(8080);
    let mut child = std::process::Command::new(exe)
        .arg("-m")
        .arg(model)
        .arg("--mmproj")
        .arg(mmproj)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("-c")
        .arg("4096")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {}", exe.display()))?;
    let health = format!("http://127.0.0.1:{port}/health");
    // ureq 3 has no per-request timeout — a tiny agent with a 2s global budget
    // keeps the poll from hanging on a half-open server.
    let probe = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(2)))
            .build(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if child.try_wait()?.is_some() {
            anyhow::bail!(
                "llama-server exited before ready (model files OK? run `webrain install vision`)"
            );
        }
        if let Ok(r) = probe.get(&health).call()
            && r.into_body()
                .read_to_string()
                .unwrap_or_default()
                .contains("ok")
        {
            break;
        }
        if std::time::Instant::now() > deadline {
            anyhow::bail!("llama-server did not become ready in 120s");
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
    Ok((
        child,
        format!("http://127.0.0.1:{port}/v1/chat/completions"),
    ))
}

/// Send up to 3 evenly-sampled frames to a vision LLM and return per-frame
/// captions + an overall visual summary as text.
/// ponytail: 3 frames = qwen3.6-27b's hard image cap AND fits the free-tier
/// 8000 TPM budget (~2.9K tokens); a longer video stays a coarse gist — the
/// full frame set is still available to vision-capable clients. Upgrade path
/// for long videos: chunk 3-frame requests + a higher-tier key.
fn describe_frames(frames: &[Frame]) -> Result<String> {
    let groq = std::env::var("GROQ_API_KEY").ok();
    let openai = std::env::var("OPENAI_API_KEY").ok();
    let local = crate::install::vision_local();
    let target = vision_target(
        groq.as_deref(),
        openai.as_deref(),
        local.as_ref().map(|t| t.1.as_path()),
        local.as_ref().map(|t| t.2.as_path()),
    )
    .ok_or_else(|| {
        anyhow!("frame vision needs GROQ_API_KEY/OPENAI_API_KEY or the local Qwen3-VL (run `webrain install vision`)")
    })?;
    // ureq 3 defaults to http_status_as_error(true), which DROPS the response
    // body on 4xx/5xx. One-shot agent (vision is opt-in/rare) with status-as-
    // error off so non-2xx returns Ok and we surface the API's own message;
    // frames also need >the shared agent's 30s timeout.
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .build(),
    );
    use base64::Engine;
    let step = (frames.len() as f64 / 3.0).ceil().max(1.0) as usize;
    let mut content: Vec<Value> = vec![
        json!({"type":"text","text":"These are frames sampled at even intervals from a video, in chronological order. For each frame, describe in one short line what is visually happening. Then give a 1-2 sentence overall summary of the video's visual content."}),
    ];
    for f in frames.iter().step_by(step).take(3) {
        let bytes = std::fs::read(&f.path)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        content.push(json!({"type":"image_url","image_url":{"url": format!("data:image/jpeg;base64,{b64}")}}));
    }
    let model_name = match &target {
        VisionTarget::Cloud { model, .. } => model.clone(),
        VisionTarget::Local { model, .. } => model
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "qwen3-vl-2b".to_string()),
    };
    let body = json!({ "model": model_name, "messages": [{"role":"user","content": content}], "max_tokens": 512 });
    let cloud = matches!(&target, VisionTarget::Cloud { .. });
    let mut result = match target {
        VisionTarget::Cloud { endpoint, key, .. } => {
            post_vision(&agent, &endpoint, Some(&key), &body)
        }
        VisionTarget::Local { model, mmproj } => {
            let (server, _, _) = local.as_ref().expect("local target implies local stack");
            let (mut child, endpoint) = spawn_llama_server(server, &model, &mmproj)?;
            let out = post_vision(&agent, &endpoint, None, &body);
            let _ = child.kill();
            let _ = child.wait();
            out
        }
    };
    // ponytail: a configured cloud provider can 429/5xx mid-watch — fall back to
    // the bundled local Qwen3-VL instead of failing the whole watch. Keeps the
    // "cloud-first, local offline" promise honest under rate limits.
    if result.is_err() && cloud {
        if let Some((server, model, mmproj)) = local.as_ref() {
            let (mut child, endpoint) = spawn_llama_server(server, model, mmproj)?;
            result = post_vision(&agent, &endpoint, None, &body);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Watch a single video (URL or local path) → transcript + frames JSON.
pub fn watch(source: &str, opts: &WatchOpts) -> Result<Value> {
    let t0 = std::time::Instant::now();
    let is_url = source.starts_with("http://") || source.starts_with("https://");
    let work = opts
        .out_dir
        .clone()
        .unwrap_or_else(|| format!("watch_{}", std::process::id()));
    let wd = Path::new(&work);
    let dl = wd.join("download");
    let _ = std::fs::create_dir_all(&dl);

    // 1) Cheap captions first (URL only).
    let mut segments: Vec<Segment> = Vec::new();
    let mut transcript_source = "none";
    if is_url {
        if let Some(vtt) = ytdlp_captions(source, &dl) {
            if let Ok(text) = std::fs::read_to_string(&vtt) {
                let segs = vtt_segments(&text);
                if !segs.is_empty() {
                    segments = segs;
                    transcript_source = "captions";
                }
            }
        }
        // Transcript detail with captions in hand → skip the video download.
        if opts.detail == Detail::Transcript && !segments.is_empty() {
            return Ok(finish_json(
                source,
                &dl,
                None,
                &probe_placeholder(),
                opts,
                &segments,
                transcript_source,
                &[],
            ));
        }
    }

    // 2) The video itself (download for URLs, use as-is for local files).
    let (video, sub_vtt): (String, Option<String>) = if is_url {
        let (v, s) = ytdlp_download(source, &dl)?;
        if transcript_source == "none" {
            if let Some(vtt) = s.clone() {
                if let Ok(text) = std::fs::read_to_string(&vtt) {
                    let segs = vtt_segments(&text);
                    if !segs.is_empty() {
                        segments = segs;
                        transcript_source = "captions";
                    }
                }
            }
        }
        (v, s)
    } else {
        if !Path::new(source).exists() {
            return Err(anyhow!("file not found: {source}"));
        }
        (source.to_string(), None)
    };
    let _ = sub_vtt;

    let probe = ffprobe(&video).unwrap_or_default();

    // 3) Frames (skip for transcript detail).
    let mut frame_list: Vec<Frame> = Vec::new();
    if opts.detail != Detail::Transcript && probe.duration > 0.0 {
        frame_list = frames(&video, wd, &probe, opts).unwrap_or_default();
    }

    // 4) Transcript: captions, else local whisper-cli, else cloud Whisper API.
    if segments.is_empty() && !opts.no_whisper && probe.has_audio {
        if let Ok(audio) = extract_audio(&video, wd) {
            // Local first — offline/private/free, GPU when present. A missing
            // binary/model falls through to the cloud API (if a key is set).
            if let Ok(segs) = whisper_local(&audio, wd)
                && !segs.is_empty()
            {
                segments = segs;
                transcript_source = "local";
            } else if opts.stt_backend == SttBackend::Whisper {
                if let Ok(segs) = whisper_transcribe(&audio, opts.stt_backend)
                    && !segs.is_empty()
                {
                    segments = segs;
                    transcript_source = "whisper";
                }
            }
        }
    }

    let mut out = finish_json(
        source,
        &dl,
        Some(&video),
        &probe,
        opts,
        &segments,
        transcript_source,
        &frame_list,
    );
    // Optional vision fallback: text captions so the LLM "sees" the frames even
    // when the client can't deliver image content (API cost — opt in only).
    if opts.vision && !frame_list.is_empty() {
        match describe_frames(&frame_list) {
            Ok(txt) => out["vision"] = json!(txt),
            Err(e) => out["vision_error"] = json!(e.to_string()),
        }
    }
    out["ms"] = json!(t0.elapsed().as_millis());
    Ok(out)
}

fn probe_placeholder() -> Probe {
    Probe::default()
}

fn finish_json(
    source: &str,
    dl: &Path,
    video: Option<&str>,
    probe: &Probe,
    opts: &WatchOpts,
    segments: &[Segment],
    transcript_source: &str,
    frames: &[Frame],
) -> Value {
    let mut v = json!({
        "source": source,
        "title": info_title(dl),
        "duration": probe.duration,
        "resolution": if probe.width > 0 { format!("{}x{}", probe.width, probe.height) } else { "unknown".to_string() },
        "has_audio": probe.has_audio,
        "detail": opts.detail.as_str(),
        "transcript_source": transcript_source,
        "transcript": segments.iter().map(|s| json!({"t": s.start, "text": s.text})).collect::<Vec<_>>(),
        "frames": frames.iter().map(|f| json!({"index": f.index, "t": f.t, "path": f.path})).collect::<Vec<_>>(),
        "work_dir": dl.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
    });
    if let Some(vid) = video {
        v["video"] = json!(vid);
    }
    v
}

/// Watch N videos in parallel (bounded worker pool). One result per source, in
/// input order; per-video failures become `{"source":…,"error":…}` entries.
pub fn watch_batch(sources: &[String], opts: &WatchOpts) -> Vec<Value> {
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let results: std::sync::Mutex<Vec<Option<Value>>> =
        (0..sources.len()).map(|_| None).collect::<Vec<_>>().into();
    let next = std::sync::Mutex::new(0usize);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = {
                        let mut g = next.lock().unwrap();
                        if *g >= sources.len() {
                            return;
                        }
                        let i = *g;
                        *g += 1;
                        i
                    };
                    let t0 = std::time::Instant::now();
                    let mut v = match watch(&sources[i], opts) {
                        Ok(v) => v,
                        Err(e) => json!({"source": sources[i], "error": e.to_string()}),
                    };
                    v["ms"] = json!(t0.elapsed().as_millis());
                    results.lock().unwrap()[i] = Some(v);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.unwrap_or_else(|| json!({"error": "watch worker failed"})))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests — the one runnable check for the non-trivial logic (VTT parsing).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vtt_parses_mmss_and_hhmmss_cues() {
        let vtt = "WEBVTT\n\n00:01.500 --> 00:03.000 align:start\nHello there\n\n01:02:03.400 --> 01:02:05.000\nSecond line\n\n00:10.000 --> 00:11.000\n\n"; // empty cue skipped
        let s = vtt_segments(vtt);
        assert_eq!(s.len(), 2, "empty cue must be skipped");
        assert!((s[0].start - 1.5).abs() < 0.01, "MM:SS.mmm start");
        assert!((s[0].end - 3.0).abs() < 0.01);
        assert_eq!(s[0].text, "Hello there");
        assert!((s[1].start - 3723.4).abs() < 0.01, "HH:MM:SS.mmm start");
        assert_eq!(s[1].text, "Second line");
    }

    #[test]
    fn frame_budget_scales_with_duration() {
        assert_eq!(frame_budget(10.0, Detail::Balanced), 20);
        assert_eq!(frame_budget(120.0, Detail::Balanced), 60);
        assert_eq!(frame_budget(3600.0, Detail::Balanced), 100);
        assert_eq!(frame_budget(3600.0, Detail::Efficient), 50);
        assert_eq!(frame_budget(10.0, Detail::Transcript), 0);
    }

    #[test]
    fn vision_target_prefers_cloud_then_local() {
        assert!(vision_target(None, None, None, None).is_none());
        match vision_target(Some("g"), Some("o"), None, None).unwrap() {
            VisionTarget::Cloud {
                endpoint,
                model,
                key,
            } => {
                assert!(endpoint.contains("groq.com"));
                assert!(model.contains("qwen"));
                assert_eq!(key, "g");
            }
            _ => panic!("groq should win"),
        }
        match vision_target(None, Some("o"), None, None).unwrap() {
            VisionTarget::Cloud {
                endpoint, model, ..
            } => {
                assert!(endpoint.contains("openai.com"));
                assert_eq!(model, "gpt-4o-mini");
            }
            _ => panic!("openai fallback"),
        }
        let (m, p) = (
            std::path::PathBuf::from("m.gguf"),
            std::path::PathBuf::from("p.gguf"),
        );
        match vision_target(None, None, Some(&m), Some(&p)).unwrap() {
            VisionTarget::Local { model, mmproj } => {
                assert_eq!(model, m);
                assert_eq!(mmproj, p);
            }
            _ => panic!("local hero fallback"),
        }
    }

    #[test]
    fn stt_provider_picks_first_set_key_and_honors_model_override() {
        // no keys → None
        assert!(stt_provider(None, None, None, None).is_none());
        // first set key wins (Groq over OpenAI/Fireworks)
        let (ep, model, key) =
            stt_provider(Some("g"), Some("o"), Some("f"), None).expect("groq wins");
        assert!(ep.contains("groq.com"));
        assert_eq!(model, "whisper-large-v3");
        assert_eq!(key, "g");
        // OpenAI when Groq unset
        let (ep, model, _) = stt_provider(None, Some("o"), Some("f"), None).expect("openai");
        assert!(ep.contains("openai.com"));
        assert_eq!(model, "whisper-1");
        // Fireworks only when it's the sole key
        let (ep, model, _) = stt_provider(None, None, Some("f"), None).expect("fireworks");
        assert!(ep.contains("fireworks.ai"));
        assert_eq!(model, "whisper-v3");
        // WEBRAIN_STT_MODEL override beats the provider default
        let (_, model, _) =
            stt_provider(Some("g"), None, None, Some("whisper-large-v3-turbo")).expect("override");
        assert_eq!(model, "whisper-large-v3-turbo");
    }

    #[test]
    fn parses_whisper_cli_json_segments() {
        // offsets (ms) preferred; empty text skipped
        let v: Value = serde_json::from_str(
            r#"{"transcription":[
                {"timestamps":{"from":"00:00:00,000","to":"00:00:02,500"},"offsets":{"from":0,"to":2500},"text":"Hello there"},
                {"timestamps":{"from":"00:00:02,500","to":"00:00:04,000"},"offsets":{"from":2500,"to":4000},"text":"World"},
                {"timestamps":{"from":"00:00:04,000","to":"00:00:05,000"},"offsets":{"from":4000,"to":5000},"text":""}
            ]}"#,
        )
        .unwrap();
        let s = whisper_json_segments(&v);
        assert_eq!(s.len(), 2, "empty text segment skipped");
        assert!((s[0].start - 0.0).abs() < 0.001);
        assert!((s[0].end - 2.5).abs() < 0.001);
        assert_eq!(s[0].text, "Hello there");
        assert!((s[1].start - 2.5).abs() < 0.001);
        // timestamps-only fallback (no offsets): "HH:MM:SS,mmm"
        let v2: Value = serde_json::from_str(
            r#"{"transcription":[{"timestamps":{"from":"00:01:30,000","to":"00:01:35,250"},"text":"Fallback"}]}"#,
        )
        .unwrap();
        let s2 = whisper_json_segments(&v2);
        assert_eq!(s2.len(), 1);
        assert!((s2[0].start - 90.0).abs() < 0.001);
        assert!((s2[0].end - 95.25).abs() < 0.001);
    }
}
