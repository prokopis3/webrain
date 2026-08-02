//! PixelRAG-style vision-embedding index.
//!
//! Tiles (`webrain_pixel` / `TileEngine`) → embed via an OpenAI-compatible
//! `/embeddings` endpoint (vLLM serving `Qwen/Qwen3-VL-Embedding-2B`, TEI,
//! Ollama, …) → cosine index → retrieve top-k by a text/image query.
//!
//! Config via env (primary = OpenAI-compatible embedder, secondary = Qwen3-VL):
//! - `EMBED_URL`   (default `https://api.openai.com/v1/embeddings`; any
//!   OpenAI-compatible server works: OpenAI, Ollama `/v1/embeddings`, TEI,
//!   SiliconFlow `https://api.siliconflow.cn/v1/embeddings`, local vLLM)
//! - `EMBED_MODEL` (default `text-embedding-3-small`)
//! - `EMBED_API_KEY` (Bearer token; required for hosted providers)
//! - `EMBED_FALLBACK_URL`/`EMBED_FALLBACK_MODEL`/`EMBED_FALLBACK_API_KEY`
//!   (default local vLLM `http://127.0.0.1:8000/v1/embeddings` +
//!   `Qwen/Qwen3-VL-Embedding-2B`) — tried only when the primary endpoint fails.
//!
//! Verified end-to-end against a local Ollama endpoint (nomic-embed-text): tiles
//! captured via Chrome CDP → data URLs → real HTTP embed → JSONL store → cosine
//! top-k. Real image semantics need the vision model (vLLM/GPU or SiliconFlow);
//! OpenRouter has no embedding models.
//!
//! ponytail: flat JSONL append store + in-memory cosine, zero new deps. Swap for
//! sqlite-vec (same shape) when one index exceeds ~a few thousand vectors.

use crate::browser::BrowserBackend;
use anyhow::Context;
use serde_json::{json, Value};
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
        let resp = req
            .send_json(body)
            .context("embedding request failed")?;
        let data: Value = serde_json::from_str(&resp.into_body().read_to_string()?)
            .context("embedding response was not JSON")?;
        let arr = data["data"].as_array().context("no embeddings in response")?;
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
    dirty: bool,
}

impl VectorStore {
    pub fn new(index: &str, dir: &str) -> Self {
        Self {
            index: index.to_string(),
            path: std::path::PathBuf::from(dir).join(format!("{index}.jsonl")),
            map: HashMap::new(),
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
                    .map(|a| a.iter().filter_map(|n| n.as_f64().map(|f| f as f32)).collect())
                    .unwrap_or_default();
                if !id.is_empty() && !vec.is_empty() {
                    self.map.insert(id, vec);
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

    pub fn len(&self) -> usize {
        self.map.len()
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
}

/// Index the current page: capture PixelRAG tiles, embed them as images, store.
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
    let inputs: Vec<EmbedInput> = tiles.iter().map(|t| EmbedInput::Image(t.png_b64.clone())).collect();
    let vecs = client.embed(&inputs)?;
    let mut store = VectorStore::new(tag, "vision");
    store.load()?;
    for (i, t) in tiles.iter().enumerate() {
        store.add(
            &format!("{url}#tile{}", t.index),
            vecs.get(i).cloned().unwrap_or_default(),
        );
    }
    let total = store.len();
    store.save()?;
    Ok(json!({
        "status": "ok", "tag": tag, "indexed": tiles.len(), "total": total,
        "url": url, "dim": vecs.first().map(|v| v.len()).unwrap_or(0),
        "model": client.model(),
        "fallback": client.fallback.as_ref().map(|f| f.model.clone())
    }))
}

/// Embed a text query and return the cosine top-k stored tile ids.
pub fn retrieve(tag: &str, query: &str, k: usize) -> anyhow::Result<Value> {
    let client = EmbeddingClient::from_env();
    let vecs = client.embed(&[EmbedInput::Text(query.to_string())])?;
    let mut store = VectorStore::new(tag, "vision");
    store.load()?;
    let q = vecs.into_iter().next().unwrap_or_default();
    let top = store.search(&q, k);
    let results: Vec<Value> = top
        .into_iter()
        .map(|(id, s)| json!({ "id": id, "score": s }))
        .collect();
    Ok(json!({ "status": "ok", "tag": tag, "total": store.len(), "results": results }))
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
