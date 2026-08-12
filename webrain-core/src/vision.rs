//! PixelRAG-style vision-embedding index.
//!
//! Tiles (`webrain_pixel` / `TileEngine`) → embed via an OpenAI-compatible
//! `/v1/embeddings` endpoint (`EMBED_*`) → cosine index → retrieve top-k by a
//! text/image query. Embeddings power RETRIEVAL; real tile understanding uses
//! the bundled LOCAL vision model (`Qwen3-VL-2B` via llama-server, `webrain
//! install vision`) — `describe_tiles` captions tiles in one batched chat call
//! (replacing the old `Qwen3-VL-Embedding-2B` @ vLLM:8000 vision fallback).
//!
//! Config via env (primary = OpenAI-compatible embedder, secondary = Qwen3-VL):
//! - `EMBED_URL`   (default `https://api.openai.com/v1/embeddings`; any
//!   OpenAI-compatible server works: OpenAI, Ollama `/v1/embeddings`, TEI,
//!   SiliconFlow `https://api.siliconflow.cn/v1/embeddings`, local vLLM)
//! - `EMBED_MODEL` (default `text-embedding-3-small`)
//! - `EMBED_API_KEY` (Bearer token; required for hosted providers)
//!
//! Verified end-to-end against a local Ollama endpoint (nomic-embed-text): tiles
//! captured via Chrome CDP → data URLs → real HTTP embed → JSONL store → cosine
//! top-k.
//!
//! ponytail: flat JSONL append store + in-memory cosine, zero new deps. Swap for
//! sqlite-vec (same shape) when one index exceeds ~a few thousand vectors.

use crate::browser::BrowserBackend;
use anyhow::Context;
use serde_json::{Value, json};
use std::collections::HashMap;

/// One input to the embedding API. Qwen3-VL-Embedding takes images as data URLs;
/// text-only models take plain strings.
pub enum EmbedInput {
    Text(String),
    /// Base64 PNG, e.g. a `webrain_pixel` tile's `png_b64`.
    Image(String),
}

impl EmbedInput {
    fn to_value(&self) -> Value {
        match self {
            EmbedInput::Text(t) => json!(t),
            EmbedInput::Image(b64) => json!(format!("data:image/png;base64,{b64}")),
        }
    }
}

/// One OpenAI-compatible `/v1/embeddings` endpoint (URL + model + optional key).
struct Endpoint {
    url: String,
    api_key: Option<String>,
    model: String,
}

impl Endpoint {
    fn from_env(prefix: &str, def_url: &str, def_model: &str) -> Self {
        let e = |k: &str| std::env::var(format!("{prefix}_{k}"));
        Self {
            url: e("URL").unwrap_or_else(|_| def_url.to_string()),
            api_key: e("API_KEY").ok(),
            model: e("MODEL").unwrap_or_else(|_| def_model.to_string()),
        }
    }

    fn embed(&self, items: &[Value]) -> anyhow::Result<Vec<Vec<f32>>> {
        let body = json!({ "model": self.model, "input": items });
        let mut req = ureq::post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }
        let resp = req.send_json(body).context("embedding request failed")?;
        let data: Value = serde_json::from_str(&resp.into_body().read_to_string()?)
            .context("embedding response was not JSON")?;
        let arr = data["data"]
            .as_array()
            .context("no embeddings in response")?;
        let mut out = Vec::with_capacity(arr.len());
        for e in arr {
            let mut v = Vec::new();
            for n in e["embedding"]
                .as_array()
                .context("embedding item missing vector")?
            {
                v.push(n.as_f64().unwrap_or(0.0) as f32);
            }
            out.push(v);
        }
        Ok(out)
    }
}

/// Primary OpenAI-compatible embedder with an optional Qwen3-VL fallback.
pub struct EmbeddingClient {
    primary: Endpoint,
    fallback: Option<Endpoint>,
}

impl EmbeddingClient {
    /// Primary = `EMBED_*` (default OpenAI); secondary = `EMBED_FALLBACK_*`
    /// (default local vLLM Qwen3-VL-Embedding-2B), tried when primary fails.
    pub fn from_env() -> Self {
        Self {
            primary: Endpoint::from_env(
                "EMBED",
                "https://api.openai.com/v1/embeddings",
                "text-embedding-3-small",
            ),
            fallback: Some(Endpoint::from_env(
                "EMBED_FALLBACK",
                "http://127.0.0.1:8000/v1/embeddings",
                "Qwen/Qwen3-VL-Embedding-2B",
            )),
        }
    }

    /// Embed a batch of text/image inputs. Returns one vector per input.
    /// Falls back to the secondary (Qwen) endpoint if the primary fails.
    /// ponytail: blocking ureq call; spawn on a task if latency ever matters.
    pub fn embed(&self, inputs: &[EmbedInput]) -> anyhow::Result<Vec<Vec<f32>>> {
        let items: Vec<Value> = inputs.iter().map(EmbedInput::to_value).collect();
        match self.primary.embed(&items) {
            Ok(v) => Ok(v),
            Err(primary_err) => match &self.fallback {
                Some(f) => f.embed(&items).with_context(|| {
                    format!("primary embed failed ({primary_err:#}); fallback also failed")
                }),
                None => Err(primary_err),
            },
        }
    }

    pub fn model(&self) -> &str {
        &self.primary.model
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
fn norm(a: &[f32]) -> f32 {
    dot(a, a).sqrt()
}

/// In-memory cosine index with JSONL append persistence (one line per record:
/// `{"id": "...", "vec": [...]}`) at `dir/{index}.jsonl`.
pub struct VectorStore {
    pub index: String,
    pub path: std::path::PathBuf,
    map: HashMap<String, Vec<f32>>,
    texts: HashMap<String, String>,
    dirty: bool,
}

impl VectorStore {
    pub fn new(index: &str, dir: &str) -> Self {
        Self {
            index: index.to_string(),
            path: std::path::PathBuf::from(dir).join(format!("{index}.jsonl")),
            map: HashMap::new(),
            texts: HashMap::new(),
            dirty: false,
        }
    }

    pub fn load(&mut self) -> anyhow::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        for line in std::fs::read_to_string(&self.path)?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                let id = v["id"].as_str().unwrap_or("").to_string();
                let vec: Vec<f32> = v["vec"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|n| n.as_f64().map(|f| f as f32))
                            .collect()
                    })
                    .unwrap_or_default();
                if !id.is_empty() && !vec.is_empty() {
                    self.map.insert(id.clone(), vec);
                }
                if let Some(text) = v["text"].as_str() {
                    if !id.is_empty() {
                        self.texts.insert(id, text.to_string());
                    }
                }
            }
        }
        self.dirty = false;
        Ok(())
    }

    pub fn add(&mut self, id: &str, vec: Vec<f32>) {
        self.map.insert(id.to_string(), vec);
        self.dirty = true;
    }

    /// Add a caption-only entry (offline mode — no embedding vector).
    pub fn add_text(&mut self, id: &str, text: &str) {
        self.texts.insert(id.to_string(), text.to_string());
        self.dirty = true;
    }

    pub fn len(&self) -> usize {
        self.map.len() + self.texts.len()
    }

    /// True when this index holds caption text (offline mode) rather than vectors.
    pub fn has_text(&self) -> bool {
        !self.texts.is_empty()
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let mut s = String::new();
        for (id, vec) in &self.map {
            s.push_str(&json!({ "id": id, "vec": vec }).to_string());
            s.push('\n');
        }
        for (id, text) in &self.texts {
            s.push_str(&json!({ "id": id, "text": text }).to_string());
            s.push('\n');
        }
        std::fs::write(&self.path, s)?;
        self.dirty = false;
        Ok(())
    }

    /// Cosine top-k over the query vector, descending by score.
    pub fn search(&self, q: &[f32], k: usize) -> Vec<(String, f32)> {
        let qn = norm(q);
        let mut scored: Vec<(String, f32)> = self
            .map
            .iter()
            .map(|(id, v)| (id.clone(), dot(v, q) / (norm(v) * qn).max(1e-8)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }

    /// Keyword top-k over caption text (offline mode): fraction of query tokens
    /// present in each caption, descending.
    pub fn search_text(&self, q: &str, k: usize) -> Vec<(String, f32)> {
        let qw: Vec<String> = q.split_whitespace().map(|w| w.to_lowercase()).collect();
        if qw.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(String, f32)> = self
            .texts
            .iter()
            .map(|(id, text)| {
                let tl = text.to_lowercase();
                let hits = qw.iter().filter(|w| tl.contains(w.as_str())).count();
                (id.clone(), hits as f32 / qw.len() as f32)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }
}

/// Ask the bundled local Qwen3-VL about the current viewport (or a clip
/// region) and return its text answer — the pixel workflow tool for captchas
/// and visual QA: screenshot → local vision → answer. Unlike `describe_tiles`
/// (fixed "describe the page" prompt) this takes an arbitrary prompt, e.g.
/// "which grid tiles contain traffic lights?". No cloud key needed.
pub async fn ask_viewport(
    backend: &impl BrowserBackend,
    prompt: &str,
    clip: Option<(f64, f64, f64, f64)>,
    tiles: &[(f64, f64, f64, f64)],
    scale: f64,
) -> anyhow::Result<String> {
    // Cloud-first provider chain: OpenRouter → OpenAI → Fireworks → Groq →
    // bundled local Qwen3-VL-2B. Try each in order — a flaky provider falls
    // through to the next instead of killing the op (live hit: OpenRouter
    // returned empty, Groq was fine).
    let openrouter = std::env::var("OPENROUTER_API_KEY")
        .ok()
        .filter(|s| !s.is_empty());
    let openai = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|s| !s.is_empty());
    let fireworks = std::env::var("FIREWORKS_API_KEY")
        .ok()
        .filter(|s| !s.is_empty());
    let groq = std::env::var("GROQ_API_KEY").ok().filter(|s| !s.is_empty());
    let local = crate::install::vision_local();
    let (s, _m, _p): (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) = local
        .as_ref()
        .map(|(s, m, p)| (s.clone(), m.clone(), p.clone()))
        .unwrap_or_else(|| {
            (
                std::path::PathBuf::new(),
                std::path::PathBuf::new(),
                std::path::PathBuf::new(),
            )
        });
    let targets = crate::video::vision_targets(
        openrouter.as_deref(),
        openai.as_deref(),
        fireworks.as_deref(),
        groq.as_deref(),
        local.as_ref().map(|t| t.1.as_path()),
        local.as_ref().map(|t| t.2.as_path()),
    );
    if targets.is_empty() {
        anyhow::bail!(
            "no vision backend — set OPENROUTER_API_KEY/OPENAI_API_KEY/FIREWORKS_API_KEY/GROQ_API_KEY or run `webrain install vision`"
        );
    }

    use base64::Engine;
    // Batch mode: multiple tile clips -> ONE request with all images in order
    // (watch `describe_frames` batching). The prompt tells the model the
    // numbering (1..N). 1 call instead of N sequential per-tile calls.
    // Capture the pixels ONCE — provider-independent. Only the model name in
    // the body differs per target, so a failed provider retries on the next.
    let content: Value = if !tiles.is_empty() {
        let mut c: Vec<Value> = vec![json!({"type":"text","text": prompt})];
        for (x, y, w, h) in tiles {
            let png = backend.screenshot_clip(*x, *y, *w, *h, scale).await?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
            c.push(json!({"type":"image_url","image_url":{"url": format!("data:image/png;base64,{b64}")}}));
        }
        json!(c)
    } else {
        let (x, y, w, h) = match clip {
            Some(c) => c,
            None => {
                let vp: Value = backend
                    .evaluate("[window.innerWidth, window.innerHeight]")
                    .await?;
                (
                    0.0,
                    0.0,
                    vp[0].as_f64().unwrap_or(1280.0).max(1.0),
                    vp[1].as_f64().unwrap_or(800.0).max(1.0),
                )
            }
        };
        let png = backend.screenshot_clip(x, y, w, h, scale).await?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        json!([{"type":"text","text": prompt}, {"type":"image_url","image_url":{"url": format!("data:image/png;base64,{b64}")}}])
    };
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .build(),
    );
    // Failover: first provider that returns content wins; try the next on error.
    let mut last_err = None;
    for t in &targets {
        let (endpoint, auth, model_name): (String, Option<String>, String) = match t {
            crate::video::VisionTarget::Cloud {
                endpoint,
                model,
                key,
            } => (endpoint.clone(), Some(key.clone()), model.clone()),
            crate::video::VisionTarget::Local { model, mmproj } => {
                let e = crate::video::llama_vision_endpoint(&s, model, mmproj)?;
                let n = model
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "qwen3-vl-2b".to_string());
                (e, None, n)
            }
        };
        let body = json!({
            "model": model_name,
            "messages": [{"role":"user","content": content.clone()}],
            "max_tokens": 512
        });
        match crate::video::post_vision(&agent, &endpoint, auth.as_deref(), &body) {
            Ok(ans) => return Ok(ans),
            Err(e) => last_err = Some(e),
        }
    }
    Err(anyhow::anyhow!(
        "vision ask: {}",
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "all vision providers failed".to_string())
    ))
}

/// Caption a batch of base64 tile PNGs with the bundled LOCAL vision model
/// (Qwen3-VL-2B via llama-server, `webrain install vision`) — the PixelRAG
/// "vision model" path (embeddings can't read pixels). One batched chat call,
/// capped at 8 evenly-sampled tiles; returns the model's page description.
pub fn describe_tiles(b64: &[String]) -> anyhow::Result<String> {
    let (server, model, mmproj) = crate::install::vision_local()
        .ok_or_else(|| anyhow::anyhow!("no local vision stack — run `webrain install vision`"))?;
    let step = (b64.len() as f64 / 8.0).ceil().max(1.0) as usize;
    let mut content: Vec<Value> = vec![json!(
        {"type":"text","text":"These are PNG tiles captured from a web page in reading order. Describe the page's visible content in 2-3 sentences."}
    )];
    for b in b64.iter().step_by(step).take(8) {
        content.push(
            json!({"type":"image_url","image_url":{"url": format!("data:image/png;base64,{b}")}}),
        );
    }
    let model_name = model
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "qwen3-vl-2b".to_string());
    let body = json!({
        "model": model_name,
        "messages": [{"role":"user","content": content}],
        "max_tokens": 512
    });
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .build(),
    );
    let endpoint = crate::video::llama_vision_endpoint(&server, &model, &mmproj)?;
    crate::video::post_vision(&agent, &endpoint, None, &body)
        .map_err(|e| anyhow::anyhow!("local vision: {e:#}"))
}

/// Caption every tile with the bundled local Qwen3-VL — one llama-server spawn
/// for all tiles, then a few BATCHED requests (like `watch` batches frames): a
/// group of tiles per request, answered as numbered per-tile captions. Batching
/// beats per-tile calls because llama-server runs a single slot (concurrency
/// would only queue). Used as the offline fallback when no embedding backend.
fn caption_tiles(b64: &[String]) -> anyhow::Result<Vec<String>> {
    let (server, model, mmproj) = crate::install::vision_local()
        .ok_or_else(|| anyhow::anyhow!("no local vision stack — run `webrain install vision`"))?;
    let endpoint = crate::video::llama_vision_endpoint(&server, &model, &mmproj)?;
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .build(),
    );
    let model_name = model
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "qwen3-vl-2b".to_string());
    let group = 4usize;
    let mut out: Vec<String> = vec![String::new(); b64.len()];
    for (start, chunk) in b64.chunks(group).enumerate() {
        let base = start * group;
        let mut content: Vec<Value> = vec![json!({
            "type":"text",
            "text": format!(
                "Caption each of these {} images on its own numbered line as '1. caption', '2. caption', ... Each under 8 words.",
                chunk.len()
            )
        })];
        for b in chunk {
            content.push(json!({"type":"image_url","image_url":{"url": format!("data:image/png;base64,{b}")}}));
        }
        let body = json!({
            "model": model_name,
            "messages": [{"role":"user","content": content}],
            "max_tokens": (32 + 24 * chunk.len()).min(512)
        });
        // ponytail: health flips to "ok" a beat before the model accepts
        // inference — a single retry after a pause absorbs that first 503.
        let mut raw = crate::video::post_vision(&agent, &endpoint, None, &body);
        if raw.is_err() {
            std::thread::sleep(std::time::Duration::from_secs(5));
            raw = crate::video::post_vision(&agent, &endpoint, None, &body);
        }
        // Surface the real error instead of silently returning empty captions
        // (" |  |  | " looked like success but was a swallowed failure).
        let raw = raw.map_err(|e| anyhow::anyhow!("local vision batch {base}: {e:#}"))?;
        for (j, cap) in parse_numbered(&raw, chunk.len()).into_iter().enumerate() {
            out[base + j] = cap;
        }
    }
    Ok(out)
}

/// Split a "1. cap / 2. cap ..." model answer into per-index captions.
fn parse_numbered(raw: &str, n: usize) -> Vec<String> {
    let mut caps: Vec<Option<String>> = vec![None; n];
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let ne = t.find(|c: char| !c.is_ascii_digit());
        if let Some(ne) = ne {
            if ne > 0 {
                if let Ok(k) = t[..ne].parse::<usize>() {
                    if (1..=n).contains(&k) {
                        let cap = t[ne..]
                            .trim_start_matches(|c| c == '.' || c == ':' || c == '-' || c == ')')
                            .trim()
                            .to_string();
                        caps[k - 1] = Some(cap);
                        continue;
                    }
                }
            }
        }
        // no leading number → fill the first empty slot positionally
        if let Some(slot) = caps.iter_mut().find(|c| c.is_none()) {
            *slot = Some(t.to_string());
        }
    }
    caps.into_iter().map(|c| c.unwrap_or_default()).collect()
}

/// Index the current page: capture PixelRAG tiles, embed them as images, store.
/// When the bundled local vision model is installed, also captions the tiles
/// (returned as `vision`) — real understanding the embeddings can't provide.
pub async fn index_current_page(
    backend: &impl BrowserBackend,
    tag: &str,
    tile_w: f64,
    tile_h: f64,
    max_tiles: usize,
) -> anyhow::Result<Value> {
    let engine = crate::engines::TileEngine::new(tile_w, tile_h, max_tiles);
    let tiles = engine.tile(backend).await?;
    if tiles.is_empty() {
        anyhow::bail!("no tiles captured — navigate first, or the page is empty");
    }
    let url = backend
        .evaluate("location.href")
        .await?
        .as_str()
        .unwrap_or("")
        .to_string();
    let client = EmbeddingClient::from_env();
    let inputs: Vec<EmbedInput> = tiles
        .iter()
        .map(|t| EmbedInput::Image(t.png_b64.clone()))
        .collect();
    let mut store = VectorStore::new(tag, "vision");
    store.load()?;
    let (mode, vision): (&str, Option<String>) = match client.embed(&inputs) {
        Ok(vecs) => {
            for (i, t) in tiles.iter().enumerate() {
                store.add(&format!("{url}#tile{}", t.index), vecs[i].clone());
            }
            let v = crate::install::vision_local()
                .is_some()
                .then(|| {
                    let t: Vec<String> = tiles.iter().map(|x| x.png_b64.clone()).collect();
                    describe_tiles(&t).ok()
                })
                .flatten();
            ("embed", v)
        }
        Err(_) => {
            // ponytail: offline fallback — caption each tile with the bundled
            // local Qwen3-VL so webrain_vision works without a cloud embed key.
            if crate::install::vision_local().is_some() {
                let b: Vec<String> = tiles.iter().map(|x| x.png_b64.clone()).collect();
                let caps = caption_tiles(&b)?;
                for (i, t) in tiles.iter().enumerate() {
                    store.add_text(&format!("{url}#tile{}", t.index), &caps[i]);
                }
                ("captions", Some(caps.join(" | ")))
            } else {
                anyhow::bail!(
                    "no embedding backend (set EMBED_URL/EMBED_API_KEY) and no local vision (run `webrain install vision`)"
                );
            }
        }
    };
    let total = store.len();
    store.save()?;
    Ok(json!({
        "status": "ok", "mode": mode, "tag": tag, "indexed": tiles.len(), "total": total,
        "url": url, "vision": vision
    }))
}

/// Embed a text query and return the cosine top-k stored tile ids.
pub fn retrieve(tag: &str, query: &str, k: usize) -> anyhow::Result<Value> {
    let mut store = VectorStore::new(tag, "vision");
    store.load()?;
    // Offline caption index (no embed backend): keyword-match over captions.
    if store.has_text() {
        let top = store.search_text(query, k);
        let results: Vec<Value> = top
            .into_iter()
            .map(|(id, s)| json!({ "id": id, "score": s }))
            .collect();
        return Ok(json!({
            "status": "ok", "mode": "captions", "tag": tag,
            "total": store.len(), "results": results
        }));
    }
    let client = EmbeddingClient::from_env();
    let vecs = client.embed(&[EmbedInput::Text(query.to_string())])?;
    let q = vecs.into_iter().next().unwrap_or_default();
    let top = store.search(&q, k);
    let results: Vec<Value> = top
        .into_iter()
        .map(|(id, s)| json!({ "id": id, "score": s }))
        .collect();
    Ok(json!({
        "status": "ok", "mode": "embed", "tag": tag,
        "total": store.len(), "results": results
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ponytail: one self-check for the non-trivial cosine store round-trip.
    #[test]
    fn store_cosine_roundtrip() {
        let dir = std::env::temp_dir().to_str().unwrap().to_string();
        let mut st = VectorStore::new("test_idx", &dir);
        st.add("a", vec![1.0, 0.0]);
        st.add("b", vec![0.0, 1.0]);
        st.save().unwrap();

        let mut st2 = VectorStore::new("test_idx", &dir);
        st2.load().unwrap();
        assert_eq!(st2.len(), 2);
        let top = st2.search(&[1.0, 0.1], 1);
        assert_eq!(top[0].0, "a");
        assert!(top[0].1 > 0.99, "cosine score was {}", top[0].1);

        let _ = std::fs::remove_file(st2.path);
    }
}
