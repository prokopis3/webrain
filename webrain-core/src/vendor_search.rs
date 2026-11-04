//! Vendor-side search for `webrain chat --search`.
//!
//! A chat turn's request is built by the Copilot runtime, and `ProviderConfig`
//! has no field for a body parameter, so `web_search_options` /
//! `tools: [{type:"web_search"}]` / `google_search` cannot be injected *into*
//! the turn. What every one of those vendors does expose is a separate
//! search-capable request that returns an answered, cited summary — so the same
//! key can be asked twice: once for grounding, once for the turn.
//!
//! Each function below is one such request. They are blocking (`ureq`, like
//! `serp`), so callers run them off the async executor.

use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

/// Search-capable endpoints can be slow (they run several queries and read
/// pages); this is per request, not per turn.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(SEARCH_TIMEOUT))
        .build()
        .new_agent()
}

/// POST a JSON body and return the parsed response, naming the vendor on error.
fn post_json(url: &str, headers: &[(&str, &str)], body: &Value) -> anyhow::Result<Value> {
    let mut req = agent()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json");
    for &(k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .send_json(body)
        .map_err(|e| anyhow::anyhow!("{url}: {e}"))?;
    let raw = resp
        .into_body()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("{url}: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| anyhow::anyhow!("{url}: response was not JSON: {e}"))
}

/// OpenAI: the Responses API with the hosted `web_search` tool. Chat Completions
/// only supports search through special search models (`gpt-5-search-api`), so
/// Responses is the way in for a normal model id.
pub fn openai_web_search(
    base_url: &str,
    key: &str,
    model: &str,
    query: &str,
) -> anyhow::Result<String> {
    let url = format!("{}/responses", base_url.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "tools": [{ "type": "web_search" }],
        "input": query,
    });
    let v = post_json(&url, &[("Authorization", &format!("Bearer {key}"))], &body)?;
    Ok(openai_answer(&v))
}

/// Anthropic: the Messages API with the `web_search_20250305` server tool. The
/// tool runs on Anthropic's side, so the response already contains the answer
/// plus its citation blocks.
pub fn anthropic_web_search(
    base_url: &str,
    key: &str,
    model: &str,
    query: &str,
) -> anyhow::Result<String> {
    let url = format!("{}/messages", base_url.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "max_tokens": 2048,
        "messages": [{ "role": "user", "content": query }],
        "tools": [{ "type": "web_search_20250305", "name": "web_search", "max_uses": 3 }],
    });
    let v = post_json(
        &url,
        &[("x-api-key", key), ("anthropic-version", "2023-06-01")],
        &body,
    )?;
    Ok(anthropic_answer(&v))
}

/// Gemini: Grounding with Google Search, on the Interactions endpoint (the
/// `google_search` tool is only available there, not on the OpenAI-compatible
/// `/v1beta/openai` surface this row's base url points at).
pub fn gemini_grounded(
    base_url: &str,
    key: &str,
    model: &str,
    query: &str,
) -> anyhow::Result<String> {
    let host = gemini_host(base_url);
    let url = format!("{host}/v1beta/interactions");
    let body = json!({
        "model": model,
        "input": query,
        "tools": [{ "type": "google_search" }],
    });
    let v = post_json(
        &url,
        &[("x-goog-api-key", key), ("Api-Revision", "2026-05-20")],
        &body,
    )?;
    Ok(gemini_answer(&v))
}

/// The Gemini row's base url is the OpenAI-compatible `/v1beta/openai` path; the
/// Interactions endpoint hangs off the host root.
fn gemini_host(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    match trimmed.find("/v1beta") {
        Some(i) => trimmed[..i].to_string(),
        None => trimmed.to_string(),
    }
}

/// `output[]` → the `message` item → its first `output_text`.
fn openai_answer(v: &Value) -> String {
    v["output"]
        .as_array()
        .and_then(|items| items.iter().find(|i| i["type"] == "message"))
        .and_then(|m| m["content"].as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// `content[]` → every `text` block, plus the result list as citations. Search
/// results themselves are dropped (they are pages, not an answer).
fn anthropic_answer(v: &Value) -> String {
    let mut text = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let cited = anthropic_sources(v);
    if !cited.is_empty() {
        text.push_str(&format!("\n\nSources: {cited}"));
    }
    text
}

fn anthropic_sources(v: &Value) -> String {
    v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "web_search_tool_result")
                .filter_map(|b| b["content"].as_array())
                .flatten()
                .filter_map(|r| r["url"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// `steps[]` → `model_output` → its text blocks.
fn gemini_answer(v: &Value) -> String {
    v["steps"]
        .as_array()
        .map(|steps| {
            steps
                .iter()
                .filter(|s| s["type"] == "model_output")
                .filter_map(|s| s["content"].as_array())
                .flatten()
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Host of a base url, for the log line and for tests.
pub fn host_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_answer_reads_the_message_item() {
        let v = serde_json::json!({
            "output": [
                { "type": "web_search_call", "status": "completed" },
                { "type": "message", "content": [
                    { "type": "output_text", "text": "Rust 1.95 shipped." }
                ]}
            ]
        });
        assert_eq!(openai_answer(&v), "Rust 1.95 shipped.");
        assert_eq!(openai_answer(&serde_json::json!({})), "");
    }

    #[test]
    fn anthropic_answer_joins_text_and_lists_sources() {
        let v = serde_json::json!({
            "content": [
                { "type": "text", "text": "It shipped." },
                { "type": "server_tool_use", "name": "web_search" },
                { "type": "web_search_tool_result", "content": [
                    { "type": "web_search_result", "url": "https://a.test/1" },
                    { "type": "web_search_result", "url": "https://b.test/2" }
                ]},
                { "type": "text", "text": "Confirmed." }
            ]
        });
        let out = anthropic_answer(&v);
        assert!(out.starts_with("It shipped.\nConfirmed."), "{out}");
        assert!(out.contains("https://a.test/1, https://b.test/2"), "{out}");
    }

    #[test]
    fn gemini_answer_reads_model_output_steps() {
        let v = serde_json::json!({
            "steps": [
                { "type": "google_search_call", "arguments": { "queries": ["x"] } },
                { "type": "model_output", "content": [{ "type": "text", "text": "Grounded." }] }
            ]
        });
        assert_eq!(gemini_answer(&v), "Grounded.");
    }

    #[test]
    fn gemini_endpoint_is_built_from_the_host_not_the_openai_shim() {
        assert_eq!(
            gemini_host("https://generativelanguage.googleapis.com/v1beta/openai"),
            "https://generativelanguage.googleapis.com"
        );
        assert_eq!(gemini_host("https://example.test"), "https://example.test");
    }
}
