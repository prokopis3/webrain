//! The chat agent's own tools: read, write, edit, search, list, and a shell —
//! one-shot or kept alive as a terminal you can talk to.
//!
//! They live here rather than in the MCP surface on purpose. `webrain mcp` stays
//! a browser server that any LLM can drive; `webrain chat` is the one mode that
//! can touch the directory it was started in, and only through these.
//!
//! Every path is resolved against that directory and refused if it lands outside
//! it. A file tool that writes anywhere is a prompt-injection target with a
//! filesystem, and the page's approval dialog is the second gate: every tool
//! here asks before it runs, exactly like an MCP call that changes state.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use copilot_sdk::Error;
use copilot_sdk::ToolResult;
use copilot_sdk::tool::ToolHandler;
use copilot_sdk::types::{Tool, ToolBinaryResult, ToolInvocation, ToolResultExpanded};
use serde_json::{Value, json};

/// A shell command that has not answered by now is a command the user did not
/// mean to run. It is killed, and the model is told so.
const SHELL_TIMEOUT: Duration = Duration::from_secs(120);

/// How much of a running job's output is kept. A dev server left on for an hour
/// prints more than anyone will read, and the tail is what shows whether it is
/// still healthy.
const KEEP: usize = 100_000;

/// How much of that the page gets per poll. The transcript is not a log viewer;
/// three screens is enough to see a job is alive and roughly on task.
const TAIL: usize = 1_500;

/// Finished jobs are kept so their output is still readable, but not forever:
/// twenty is more than a session's worth and bounds the memory at 2 MB.
const KEEP_DEAD: usize = 20;

/// The names the session's tool allowlist has to carry, in the order the prompt
/// lists them. A box in the page's tools dialog is checked for each of these.
pub(crate) const NAMES: [&str; 10] = [
    "read_file",
    "write_file",
    "list_dir",
    "edit_file",
    "search_files",
    "view_image",
    "web_search",
    "web_fetch",
    "run_shell",
    "terminal",
];

/// Where "search the web" and "read this page" go.
///
/// DeepSeek sells both as plain JSON APIs on the key the session already holds
/// (`/v1/tools/search`, `/v1/tools/fetch` — 0.75 USD per 1000 successful calls),
/// and their *server-side* web tools are not usable here: the runtime builds the
/// request, so we cannot inject a vendor-native tool declaration. A host tool
/// calling the JSON endpoints is the only way in, which is what this is. Every
/// other provider falls back to webrain's own engines.
pub(crate) struct Web {
    pub(crate) base: String,
    pub(crate) key: String,
}

impl Web {
    fn deepseek(&self) -> Option<String> {
        self.base
            .contains("deepseek.com")
            .then(|| self.base.trim_end_matches('/').to_string())
    }
}

/// The tools, with their schemas and their handlers attached. `web` carries the
/// session's provider so the two web tools can prefer its own search.
pub(crate) fn workspace_tools(web: Option<Web>) -> Vec<Tool> {
    let web = web.map(std::sync::Arc::new);
    vec![
        tool(
            "read_file",
            "Read a UTF-8 text file. Lines are 1-based and numbered in the \
             output; omit them to read the whole file.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "path relative to the working directory"},
                    "start_line": {"type": "integer", "description": "first line to return, 1-based"},
                    "end_line": {"type": "integer", "description": "last line to return, inclusive"}
                },
                "required": ["path"]
            }),
            ReadFile,
        ),
        tool(
            "write_file",
            "Write a UTF-8 text file, replacing it if it exists. The parent \
             directory must already exist.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "path relative to the working directory"},
                    "content": {"type": "string", "description": "the whole new contents"}
                },
                "required": ["path", "content"]
            }),
            WriteFile,
        ),
        tool(
            "list_dir",
            "List a directory: one entry per line, `/` after a directory's name \
             and its size in bytes after a file's.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "path relative to the working directory, default `.`"}
                }
            }),
            ListDir,
        ),
        tool(
            "edit_file",
            "Replace an exact string in a file. Refuses a string that matches \
             nothing or more than one place, so it cannot edit the wrong line by \
             accident; use write_file to replace a whole file.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "path relative to the working directory"},
                    "old_string": {"type": "string", "description": "the exact text to replace, whitespace and indentation included"},
                    "new_string": {"type": "string", "description": "the replacement text"},
                    "replace_all": {"type": "boolean", "description": "replace every occurrence; default false"}
                },
                "required": ["path", "old_string", "new_string"]
            }),
            EditFile,
        ),
        tool(
            "search_files",
            "Search the working directory with a regular expression, one match \
             per line as `path:line: text`. Skips `.git`, `target` and \
             `node_modules`.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "regular expression matched against each line"},
                    "path": {"type": "string", "description": "directory to search, default `.`"},
                    "max_results": {"type": "integer", "description": "stop after this many matching lines, default 200, max 500"}
                },
                "required": ["pattern"]
            }),
            SearchFiles,
        ),
        // VS Code's chat gets this from the extension (`ViewImageTool`, gated by
        // `chat.tools.viewImage.enabled`), not from the CLI runtime: the model
        // cannot open a picture with a text tool, and no runtime built-in
        // carries bytes back. `ToolResult::Expanded` is the only shape in this
        // SDK that can, so this one tool answers with an image content part.
        tool(
            "view_image",
            "Look at an image file and see it: png, jpg, gif, webp, bmp or svg. \
             This is the only way to read a screenshot, chart or diagram — \
             read_file gives you its bytes, not its content.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "path relative to the working directory"}
                },
                "required": ["path"]
            }),
            ViewImage,
        ),
        tool(
            "web_search",
            "Search the live web. Returns a ranked list of title, url and summary \
             — the thing to do before claiming anything about current prices, \
             releases or news.",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "the search query"},
                    "count": {"type": "integer", "description": "how many results, default 5, max 20"}
                },
                "required": ["query"]
            }),
            WebSearch(web.clone()),
        ),
        tool(
            "web_fetch",
            "Read one web page and get it back as Markdown text. Use it after \
             web_search, or for a url the user pasted.",
            json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "an http or https url"}
                },
                "required": ["url"]
            }),
            WebFetch(web.clone()),
        ),
        tool(
            "run_shell",
            "Run a shell command and return its combined output. `cmd /C` on \
             Windows, `sh -c` elsewhere. Killed after 120 seconds.",
            json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "the command line to run"},
                    "cwd": {"type": "string", "description": "working directory, default `.`"}
                },
                "required": ["command"]
            }),
            RunShell,
        ),
        tool(
            "terminal",
            "A shell that stays alive between calls — the way to run something \
             long. `start` spawns it (the platform shell unless `command` says \
             otherwise) and `send` types a line and returns what it printed — so \
             a prompt, a REPL or a dev server is answerable. `read` returns only \
             the output since the last call, and ends with the exit code once the \
             job is over; `list` names every job this session still knows so an \
             id is not lost between turns; `stop` kills it. Omit `id` to mean the \
             newest terminal. The user sees the same jobs in `/jobs`.",
            json!({
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": ["start", "send", "read", "list", "stop"],
                           "description": "start | send a line and read the reply | read new output | list the jobs | kill it"},
                    "id": {"type": "integer", "description": "terminal id from `start` or `list`; omit for the newest"},
                    "command": {"type": "string", "description": "start: program to run, default the platform shell"},
                    "input": {"type": "string", "description": "send: the line to type, Enter is added"},
                    "cwd": {"type": "string", "description": "start: working directory, default `.`"}
                },
                "required": ["op"]
            }),
            Terminal,
        ),
    ]
}

/// One declaration with its runtime behaviour. `skip_permission` stays false: the
/// page's modal is the gate that makes a shell tool safe to ship.
fn tool(name: &str, description: &str, schema: Value, handler: impl ToolHandler) -> Tool {
    let mut t = Tool::default();
    t.name = name.to_string();
    t.description = description.to_string();
    // The field is an ordered map, so the schema goes through the same JSON the
    // tool publishes — no second shape to keep in sync.
    t.parameters = serde_json::from_value(schema).unwrap_or_default();
    t.skip_permission = false;
    t.with_handler(std::sync::Arc::new(handler))
}

/// The directory every path is resolved against, and the boundary of what these
/// tools may touch.
fn root() -> Result<PathBuf, String> {
    std::fs::canonicalize(".").map_err(|e| format!("working directory: {e}"))
}

/// Resolve `p` under the working directory, refusing anything that escapes it —
/// lexically first, so a path that does not exist yet is still checked, and then
/// through symlinks, so a link out of the tree is not a way out.
fn inside(p: &str) -> Result<PathBuf, String> {
    resolved(&root()?, p)
}

/// Is `p` inside `root`? The same rule `inside` enforces, asked as a yes/no by
/// `chat_hooks` so a built-in tool cannot be approved past the fence.
pub(crate) fn within(root: &Path, p: &str) -> bool {
    resolved(root, p).is_ok()
}

/// The fence itself, with its reason — one implementation for both callers.
fn resolved(root: &Path, p: &str) -> Result<PathBuf, String> {
    let want = Path::new(if p.trim().is_empty() { "." } else { p });
    let joined = if want.is_absolute() {
        want.to_path_buf()
    } else {
        root.join(want)
    };
    let norm = lexically(&joined);
    if !norm.starts_with(root) {
        return Err(format!("refusing {p}: outside {}", root.display()));
    }
    match std::fs::canonicalize(&norm) {
        Ok(real) if real.starts_with(root) => Ok(real),
        Ok(real) => Err(format!("refusing {p}: resolves to {}", real.display())),
        Err(_) => Ok(norm),
    }
}

fn lexically(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

struct ReadFile;

#[async_trait::async_trait]
impl ToolHandler for ReadFile {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let path = inv.arguments["path"].as_str().unwrap_or_default();
        let from = inv.arguments["start_line"].as_u64();
        let to = inv.arguments["end_line"].as_u64();
        Ok(ToolResult::Text(shown(read(path, from, to))))
    }
}

struct WriteFile;

#[async_trait::async_trait]
impl ToolHandler for WriteFile {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let path = inv.arguments["path"].as_str().unwrap_or_default();
        let content = inv.arguments["content"].as_str().unwrap_or_default();
        Ok(ToolResult::Text(shown(write(path, content))))
    }
}

struct ListDir;

#[async_trait::async_trait]
impl ToolHandler for ListDir {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let path = inv.arguments["path"].as_str().unwrap_or(".");
        Ok(ToolResult::Text(shown(list(path))))
    }
}

struct ViewImage;

#[async_trait::async_trait]
impl ToolHandler for ViewImage {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let path = inv.arguments["path"].as_str().unwrap_or_default();
        Ok(view(path))
    }
}

/// The bytes, as an image the model can look at. A refusal is text, because a
/// failed call has nothing to show.
fn view(path: &str) -> ToolResult {
    let p = match inside(path) {
        Ok(p) => p,
        Err(e) => return ToolResult::Text(e),
    };
    let Some(mime) = image_mime(&p) else {
        return ToolResult::Text(format!(
            "{path} is not an image (png, jpg, gif, webp, bmp, svg) — use read_file for text"
        ));
    };
    // The provider bills and buffers these: a 40 MB TIFF is a turn's whole budget
    // and some APIs reject the request outright.
    const CAP: u64 = 8 * 1024 * 1024;
    match std::fs::metadata(&p) {
        Ok(m) if m.len() > CAP => {
            return ToolResult::Text(format!(
                "{path} is {} KB — too large to view (limit {} MB)",
                m.len() / 1024,
                CAP / (1024 * 1024)
            ));
        }
        Ok(_) => {}
        Err(e) => return ToolResult::Text(format!("{path}: {e}")),
    }
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) => return ToolResult::Text(format!("{path}: {e}")),
    };
    image_result(path, bytes, mime)
}

/// The bytes as an image content part. Split out because the fence correctly
/// refuses the OS temp dir, so a test cannot reach this through `view`.
fn image_result(path: &str, bytes: Vec<u8>, mime: &str) -> ToolResult {
    let kb = bytes.len() / 1024;
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    ToolResult::Expanded(
        ToolResultExpanded::new(format!("viewed {path} ({kb} KB, {mime})"), "success")
            .with_binary_results(vec![ToolBinaryResult {
                data,
                mime_type: mime.to_string(),
                r#type: "image".to_string(),
                description: Some(path.to_string()),
            }])
            .with_session_log(format!("viewed {path}")),
    )
}

/// An image by extension. The content is not sniffed: a file whose name says
/// `.png` is handed over as one, which is what the model is told it is.
fn image_mime(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

struct WebSearch(Option<Arc<Web>>);

#[async_trait::async_trait]
impl ToolHandler for WebSearch {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let query = inv.arguments["query"].as_str().unwrap_or_default();
        let count = inv.arguments["count"].as_u64().unwrap_or(5).clamp(1, 20) as usize;
        Ok(ToolResult::Text(
            web_results(self.0.as_deref(), query, count).await,
        ))
    }
}

struct WebFetch(Option<Arc<Web>>);

#[async_trait::async_trait]
impl ToolHandler for WebFetch {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let url = inv.arguments["url"].as_str().unwrap_or_default();
        Ok(ToolResult::Text(read_page(self.0.as_deref(), url).await))
    }
}

/// Results as the model reads them: title, url, summary — never raw JSON.
async fn web_results(web: Option<&Web>, query: &str, count: usize) -> String {
    if query.trim().is_empty() {
        return "a search with no query".to_string();
    }
    if let Some(api) = web.and_then(Web::deepseek) {
        let key = web.map(|w| w.key.clone()).unwrap_or_default();
        let body = json!({"query": query, "count": count, "need_content": false});
        match json_post(&format!("{api}/v1/tools/search"), &key, body).await {
            Ok(v) => {
                let mut out = String::new();
                for (i, r) in v["results"].as_array().into_iter().flatten().enumerate() {
                    let title = r["title"].as_str().unwrap_or("(untitled)");
                    let url = r["url"].as_str().unwrap_or("");
                    let summary = r["summary"].as_str().unwrap_or("");
                    out.push_str(&format!("{}. {title}\n   {url}\n   {summary}\n", i + 1));
                }
                if out.is_empty() {
                    "no results".to_string()
                } else {
                    out
                }
            }
            // A vendor failure is not a reason to answer from memory: fall back
            // to the engines that need no key at all.
            Err(e) => {
                let alt = serp_results(query, count).await;
                format!("deepseek search failed ({e}); webrain serp instead:\n\n{alt}")
            }
        }
    } else {
        serp_results(query, count).await
    }
}

/// webrain's own engines, over pooled HTTP — no browser, no key.
async fn serp_results(query: &str, count: usize) -> String {
    let opts = webrain_core::serp::SerpOpts {
        query: query.to_string(),
        limit: count,
        http_only: true,
        ..Default::default()
    };
    match webrain_core::serp::serp_search(&opts, None::<&webrain_core::backends::cdp::CdpBackend>)
        .await
    {
        Ok(r) if !r.results.is_empty() => r
            .results
            .iter()
            .enumerate()
            .map(|(i, x)| format!("{}. {}\n   {}\n", i + 1, x.title, x.url))
            .collect(),
        Ok(_) => "no results".to_string(),
        Err(e) => format!("search failed: {e}"),
    }
}

/// One page as Markdown, capped — a 2 MB article is not a turn's budget.
async fn read_page(web: Option<&Web>, url: &str) -> String {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return format!("{url:?} is not an http or https url");
    }
    if let Some(api) = web.and_then(Web::deepseek) {
        let key = web.map(|w| w.key.clone()).unwrap_or_default();
        match json_post(&format!("{api}/v1/tools/fetch"), &key, json!({"url": url})).await {
            Ok(v) => {
                let title = v["title"].as_str().unwrap_or("");
                let body = v["content"].as_str().unwrap_or("");
                if body.is_empty() {
                    return "that page had no readable content".to_string();
                }
                return clip_bytes(&format!("# {title}\n\n{body}"), 40_000);
            }
            Err(e) => return format!("fetch failed: {e}"),
        }
    }
    match webrain_core::engines::http_get_accept(
        url,
        // Ask for markdown first: a docs site that publishes it sends it, and a
        // page that ignores the header still arrives as HTML to be stripped.
        Some("text/markdown, text/plain;q=0.9, text/html;q=0.8"),
        None,
    ) {
        Ok((status, ctype, body)) => {
            let text = if ctype.contains("html") {
                html_to_text(&body)
            } else {
                body
            };
            clip_bytes(&format!("[{status} {ctype}]\n\n{text}"), 40_000)
        }
        Err(e) => format!("fetch failed: {e}"),
    }
}

/// Just enough HTML to text: the model needs the words, not the markup. Scripts
/// and styles go first (their contents are not prose), then every tag, then the
/// entities that actually appear in text.
fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    // Drop <script>…</script> and friends whole, in place: their contents are
    // not prose, and stripping their tags alone would leave the code behind.
    for tag in ["script", "style", "noscript", "svg"] {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");
        while let Some(i) = s.to_ascii_lowercase().find(&open) {
            let after = i + open.len();
            let end = s[after..]
                .to_ascii_lowercase()
                .find(&close)
                .map(|j| after + j + close.len())
                .unwrap_or(s.len());
            s.replace_range(i..end, " ");
        }
    }
    let mut out = String::with_capacity(s.len() / 2);
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    // Collapse the runs of blank space a stripped document is mostly made of.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n\n[truncated at {max} bytes]", &s[..cut])
}

/// One JSON POST on the session's key. Blocking client, so it runs on the
/// blocking pool: a `ureq` call on the runtime's own thread would stall the turn.
async fn json_post(url: &str, key: &str, body: Value) -> Result<Value, String> {
    let url = url.to_string();
    let key = key.to_string();
    tokio::task::spawn_blocking(move || {
        ureq::post(&url)
            .header("authorization", &format!("Bearer {key}"))
            .header("content-type", "application/json")
            .send_json(&body)
            .map_err(|e| e.to_string())?
            .body_mut()
            .read_json::<Value>()
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| Err(format!("task failed: {e}")))
}

struct RunShell;

#[async_trait::async_trait]
impl ToolHandler for RunShell {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let command = inv.arguments["command"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let cwd = inv.arguments["cwd"].as_str().unwrap_or(".").to_string();
        // A shell blocks for as long as the command runs; the async task is not
        // the one that should wait for it.
        let out = tokio::task::spawn_blocking(move || shell(&command, &cwd)).await;
        Ok(ToolResult::Text(match out {
            Ok(r) => shown(r),
            Err(e) => format!("error: {e}"),
        }))
    }
}

struct EditFile;

#[async_trait::async_trait]
impl ToolHandler for EditFile {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let path = inv.arguments["path"].as_str().unwrap_or_default();
        let old = inv.arguments["old_string"].as_str().unwrap_or_default();
        let new = inv.arguments["new_string"].as_str().unwrap_or_default();
        let all = inv.arguments["replace_all"].as_bool().unwrap_or(false);
        Ok(ToolResult::Text(shown(edit(path, old, new, all))))
    }
}

struct SearchFiles;

#[async_trait::async_trait]
impl ToolHandler for SearchFiles {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let pattern = inv.arguments["pattern"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let path = inv.arguments["path"].as_str().unwrap_or(".").to_string();
        let max = inv.arguments["max_results"].as_u64().unwrap_or(200) as usize;
        // Reading the tree blocks; the async task is not the one that should wait.
        let out = tokio::task::spawn_blocking(move || search(&pattern, &path, max)).await;
        Ok(ToolResult::Text(match out {
            Ok(r) => shown(r),
            Err(e) => format!("error: {e}"),
        }))
    }
}

/// Replace an exact substring, refusing the two ways a blind replace ruins a
/// file: a string that matches nothing (a typo, or text the model invented) and
/// one that matches several places (it meant one). Both are read-only failures,
/// so the model gets told instead of the file getting mangled.
fn edit(path: &str, old: &str, new: &str, all: bool) -> Result<String, String> {
    if old.is_empty() {
        return Err("old_string is empty — use write_file to replace a whole file".to_string());
    }
    let p = inside(path)?;
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let n = text.matches(old).count();
    if n == 0 {
        return Err(format!(
            "{}: no match — read the file and copy the text exactly",
            p.display()
        ));
    }
    if n > 1 && !all {
        return Err(format!(
            "{}: matches {n} places — include more context, or pass replace_all",
            p.display()
        ));
    }
    let at = text.find(old).unwrap_or(0);
    let line = 1 + text[..at].matches('\n').count();
    remember(&p, Some(text.clone()));
    std::fs::write(&p, text.replacen(old, new, n)).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(format!(
        "edited {}: {n} replacement{} at line {line}\n{}",
        p.display(),
        if n == 1 { "" } else { "s" },
        delta(old, new)
    ))
}

/// The change, as the tool row shows it: what `old` became.
///
/// The model wrote both strings, but the person reading the turn did not, and
/// this row is the only place it is visible before the file is already different.
fn delta(old: &str, new: &str) -> String {
    /// Lines of each side worth rendering — a whole-file move is not a reading
    /// task, and the row would be taller than the answer.
    const LINES: usize = 12;
    let mut out = String::new();
    for (sign, text) in [('-', old), ('+', new)] {
        for line in text.lines().take(LINES) {
            out.push_str(&format!("{sign} {line}\n"));
        }
        if text.lines().nth(LINES).is_some() {
            out.push_str(&format!("{sign} …\n"));
        }
    }
    out.trim_end().to_string()
}

/// Trees whose contents are generated or vendored: walking them turns a
/// two-line answer into ten thousand lines.
const SKIP_DIRS: [&str; 3] = [".git", "target", "node_modules"];
/// Bigger than this is data, not source: a `read_to_string` on it is a stall.
const MAX_FILE: u64 = 2_000_000;

fn search(pattern: &str, path: &str, max: usize) -> Result<String, String> {
    let re = regex::Regex::new(pattern).map_err(|e| format!("bad pattern: {e}"))?;
    let root = root()?;
    let start = inside(path)?;
    let mut out = Vec::new();
    walk(&start, &root, &re, max.clamp(1, 500), &mut out);
    Ok(if out.is_empty() {
        format!("no match for `{pattern}` under {}", start.display())
    } else {
        out.join("\n")
    })
}

fn walk(dir: &Path, root: &Path, re: &regex::Regex, max: usize, out: &mut Vec<String>) {
    if out.len() >= max {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // A symlink is a way out of the tree and a way to loop forever.
        if kind.is_symlink() {
            continue;
        }
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if kind.is_dir() {
            if !SKIP_DIRS.contains(&name.as_ref()) {
                walk(&p, root, re, max, out);
            }
            continue;
        }
        if entry.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE {
            continue;
        }
        // Unreadable or not UTF-8 is a binary file: nothing to grep in it.
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if !re.is_match(line) {
                continue;
            }
            out.push(format!("{}:{}: {}", relative(root, &p), i + 1, clip(line)));
            if out.len() >= max {
                return;
            }
        }
    }
}

fn relative(root: &Path, p: &Path) -> String {
    // One separator in the output whatever the host, so a match line reads the
    // same on every platform the binary ships to.
    p.strip_prefix(root)
        .unwrap_or(p)
        .display()
        .to_string()
        .replace('\\', "/")
}

/// One match per line has to stay one line, or a minified bundle is the answer.
fn clip(line: &str) -> String {
    let t = line.trim();
    if t.chars().count() <= 240 {
        return t.to_string();
    }
    format!("{}…", t.chars().take(240).collect::<String>())
}

struct Terminal;

#[async_trait::async_trait]
impl ToolHandler for Terminal {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let a = &inv.arguments;
        let op = a["op"].as_str().unwrap_or_default().to_string();
        let id = a["id"].as_u64();
        let command = a["command"].as_str().map(str::to_string);
        let input = a["input"].as_str().unwrap_or_default().to_string();
        let cwd = a["cwd"].as_str().unwrap_or(".").to_string();
        // Every op blocks (spawn, write, sleep, wait); none of them belongs on
        // the runtime's async task.
        let out = tokio::task::spawn_blocking(move || {
            terminal(&op, id, command.as_deref(), &input, &cwd)
        })
        .await;
        Ok(ToolResult::Text(match out {
            Ok(r) => shown(r),
            Err(e) => format!("error: {e}"),
        }))
    }
}

/// What one process has printed, and how much of it the model has been shown.
///
/// Two readers with different needs share this: the model wants only what is
/// new (a replay would cost more context than the tool saves) and the page wants
/// a tail of everything (it polls while its list is open). A cursor per reader
/// is cheaper than two buffers and cannot get them out of order.
#[derive(Default)]
struct Log {
    text: String,
    /// Byte offset into `text` up to which the model has been shown, so
    /// `terminal read` stays "new output only" without throwing bytes away.
    shown: usize,
}

/// A shell that outlives the call that started it. The model's handle is the
/// number; the process lives here.
struct Term {
    /// The program, as the page lists it — the registry does not keep the
    /// arguments, and a label is what a reader needs to tell two jobs apart.
    label: String,
    child: Child,
    stdin: ChildStdin,
    out: Arc<Mutex<Log>>,
}

#[derive(Default)]
struct Sessions {
    next: u64,
    all: HashMap<u64, Term>,
}

/// One registry for the process: the whole point is that a terminal survives the
/// call that made it, and `run_shell` cannot — it returns when the command does.
fn sessions() -> &'static Mutex<Sessions> {
    static S: OnceLock<Mutex<Sessions>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Sessions::default()))
}

/// Where a finished job is announced. This module has no bus of its own — one
/// process has one event stream and it belongs to the server — so `chat serve`
/// installs its publisher once, at boot.
static ANNOUNCE: OnceLock<Box<dyn Fn(String) + Send + Sync>> = OnceLock::new();

/// Install the publisher. Called once; a second call is ignored rather than
/// panicking, because the only caller is the server's own startup.
pub(crate) fn watch_exit(f: impl Fn(String) + Send + Sync + 'static) {
    let _ = ANNOUNCE.set(Box::new(f));
}

/// Nobody asks a process whether it is still alive, so one thread per job does
/// nothing but ask. Without it a build that finishes while the page is idle is
/// only discovered the next time somebody polls — and "did it finish" is the
/// question a background job exists to answer.
fn announce_when_over(id: u64, label: String) {
    let Some(tell) = ANNOUNCE.get() else {
        return;
    };
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(400));
            let exit = {
                let mut s = sessions().lock().unwrap();
                match s.all.get_mut(&id) {
                    Some(t) => t.exit(),
                    // Stopped from the page: that path already told the reader, and
                    // the entry is gone.
                    None => return,
                }
            };
            if let Some(code) = exit {
                tell(
                    json!({
                        "type": "webrain.job",
                        "data": { "id": id, "command": label, "exit": code, "running": false },
                    })
                    .to_string(),
                );
                return;
            }
        }
    });
}

fn terminal(
    op: &str,
    id: Option<u64>,
    command: Option<&str>,
    input: &str,
    cwd: &str,
) -> Result<String, String> {
    match op {
        "start" => term_start(command, cwd),
        "send" => {
            let (id, out) = {
                let mut s = sessions().lock().unwrap();
                let id = which(&s, id)?;
                let t = s.all.get_mut(&id).expect("which() returns a live id");
                t.stdin
                    .write_all(format!("{input}\n").as_bytes())
                    .and_then(|_| t.stdin.flush())
                    .map_err(|e| format!("terminal {id}: {e}"))?;
                (id, t.out.clone())
            };
            // A prompt answers at once and a build does not; wait for the quiet
            // moment rather than handing back a blank screen.
            std::thread::sleep(Duration::from_millis(400));
            Ok(format!("terminal {id}\n{}", taken(&out)))
        }
        "read" => {
            let (out, exit) = {
                let mut s = sessions().lock().unwrap();
                let id = which(&s, id)?;
                let t = s.all.get_mut(&id).expect("which() returns a live id");
                (t.out.clone(), t.exit())
            };
            let text = taken(&out);
            // A job that ended has to say so, or the model polls a dead process
            // for output that will never come.
            Ok(match exit {
                Some(code) => format!("{text}\n{}", ended(code)),
                None => text,
            })
        }
        "stop" => {
            let (id, mut t) = {
                let mut s = sessions().lock().unwrap();
                let id = which(&s, id)?;
                (id, s.all.remove(&id).expect("which() returns a live id"))
            };
            let _ = t.child.kill();
            let _ = t.child.wait();
            Ok(format!("terminal {id} stopped\n{}", taken(&t.out)))
        }
        // The registry outlives the context window: a job started forty turns ago
        // is still running, and without this the model cannot name it again.
        "list" => {
            let rows: Vec<String> = jobs()
                .iter()
                .map(|j| {
                    let how = if j["running"] == true {
                        "running".to_string()
                    } else {
                        ended(j["exit"].as_i64().map(|c| c as i32))
                    };
                    format!(
                        "#{} {how} · {} · {} bytes",
                        j["id"],
                        j["command"].as_str().unwrap_or("?"),
                        j["bytes"].as_u64().unwrap_or_default()
                    )
                })
                .collect();
            Ok(if rows.is_empty() {
                "no jobs — op `start` launches one that outlives the call".to_string()
            } else {
                rows.join("\n")
            })
        }
        other => Err(format!(
            "unknown op `{other}`: start | send | read | list | stop"
        )),
    }
}

/// The id a call means: the one named, else the newest — one shell open should
/// not require remembering its number.
fn which(s: &Sessions, id: Option<u64>) -> Result<u64, String> {
    match id {
        Some(n) if s.all.contains_key(&n) => Ok(n),
        Some(n) => Err(format!("no terminal {n}")),
        None => s
            .all
            .keys()
            .copied()
            .max()
            .ok_or_else(|| "no terminal running — op `start` first".to_string()),
    }
}

impl Term {
    /// `Some(code)` once the process is over, `None` while it runs. `try_wait`
    /// reports the same status on every later call, so asking repeatedly is
    /// safe — which is what lets a snapshot be the only thing that notices.
    fn exit(&mut self) -> Option<Option<i32>> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.code()),
            _ => None,
        }
    }
}

/// What "over" reads like. A code is a fact; a signal is the absence of one.
fn ended(code: Option<i32>) -> String {
    match code {
        Some(c) => format!("exited with code {c}"),
        None => "exited after a signal".to_string(),
    }
}

/// Every process this session started, oldest first: what the page's job list
/// shows. Reading it consumes nothing, so a snapshot and a model read never
/// race for the same bytes.
pub(crate) fn jobs() -> Vec<Value> {
    let mut s = sessions().lock().unwrap();
    let mut ids: Vec<u64> = s.all.keys().copied().collect();
    ids.sort_unstable();
    ids.into_iter()
        .map(|id| {
            let t = s.all.get_mut(&id).expect("the id came from this map");
            let exit = t.exit();
            let running = exit.is_none();
            let log = t.out.lock().unwrap();
            json!({
                "id": id,
                "command": t.label,
                "pid": t.child.id(),
                "running": running,
                "exit": exit.flatten(),
                // Bytes, not characters: the number answers "did it print
                // anything", it is not a byte-accurate reader.
                "bytes": log.text.len(),
                "output": last(&log.text, TAIL),
            })
        })
        .collect()
}

/// Stop a job by id, on the page's behalf. Unlike `terminal stop` this leaves the
/// unread output in the log: the model did not ask to read it, so stopping a job
/// from the page must not eat bytes the model has not been shown.
pub(crate) fn job_stop(id: u64) -> Result<String, String> {
    let mut t = {
        let mut s = sessions().lock().unwrap();
        let id = which(&s, Some(id))?;
        s.all.remove(&id).expect("which() returns a live id")
    };
    let _ = t.child.kill();
    let _ = t.child.wait();
    Ok(format!("stopped {}", t.label))
}

/// The last `n` bytes, cut on a character boundary — half a multi-byte
/// character is a broken glyph in the page.
fn last(text: &str, n: usize) -> String {
    if text.len() <= n {
        return text.to_string();
    }
    let from = text.len() - n;
    let from = (from..text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    text[from..].to_string()
}

fn term_start(command: Option<&str>, cwd: &str) -> Result<String, String> {
    let dir = inside(cwd)?;
    let program = command.unwrap_or(if cfg!(windows) { "cmd.exe" } else { "sh" });
    let mut cmd = Command::new(program);
    cmd.current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{program}: {e}"))?;
    let stdin = child.stdin.take().ok_or("the shell has no stdin")?;
    let out = Arc::new(Mutex::new(Log::default()));
    if let Some(pipe) = child.stdout.take() {
        drain(pipe, out.clone());
    }
    if let Some(pipe) = child.stderr.take() {
        drain(pipe, out.clone());
    }
    let mut s = sessions().lock().unwrap();
    // Finished jobs stay readable, but not forever: this process can run for
    // hours and every `start` would otherwise leave its log on the heap.
    let mut dead: Vec<u64> = s.all.keys().copied().collect();
    dead.retain(|id| s.all.get_mut(id).is_some_and(|t| t.exit().is_some()));
    if dead.len() > KEEP_DEAD {
        dead.sort_unstable();
        for id in dead.drain(..dead.len() - KEEP_DEAD) {
            s.all.remove(&id);
        }
    }
    s.next += 1;
    let id = s.next;
    let label = program.to_string();
    s.all.insert(
        id,
        Term {
            label: label.clone(),
            child,
            stdin,
            out: out.clone(),
        },
    );
    drop(s);
    announce_when_over(id, label);
    // A shell prints its banner; without the pause the first read looks empty.
    std::thread::sleep(Duration::from_millis(300));
    Ok(format!(
        "terminal {id} running `{program}` in {}\n{}",
        dir.display(),
        taken(&out)
    ))
}

/// Both pipes feed one buffer, so the model reads the screen in the order a
/// person would, and nothing blocks the caller while a command runs.
fn drain<R: Read + Send + 'static>(mut pipe: R, out: Arc<Mutex<Log>>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
            let mut log = out.lock().unwrap();
            log.text.push_str(&String::from_utf8_lossy(&buf[..n]));
            // shortcut: the last 100 KB of a chatty process, not the whole log;
            // a longer tail is heap nobody reads.
            if log.text.len() > KEEP {
                let cut = log.text.len() - KEEP;
                let cut = (cut..log.text.len())
                    .find(|i| log.text.is_char_boundary(*i))
                    .unwrap_or(log.text.len());
                log.text.drain(..cut);
                // `shown` is an offset into this same string: a trim that ignored
                // it would make the next read replay from the middle of a line or
                // panic on a stale index.
                log.shown = log.shown.saturating_sub(cut);
            }
        }
    });
}

/// New output only: a terminal that replayed its scrollback on every call would
/// cost more context than the tool saves. The bytes stay in the log — this moves
/// the cursor, it does not drain — so the page can still read the tail after the
/// model has seen everything.
fn taken(out: &Arc<Mutex<Log>>) -> String {
    let mut log = out.lock().unwrap();
    let new = log.text[log.shown..].to_string();
    log.shown = log.text.len();
    if new.is_empty() {
        "(no new output)".to_string()
    } else {
        new
    }
}

/// Both branches end up here: a tool that failed says so in the text the model
/// reads, which is what lets it correct course instead of retrying blind.
fn shown(r: Result<String, String>) -> String {
    match r {
        Ok(t) => t,
        Err(e) => format!("error: {e}"),
    }
}

/// How many file changes can be put back. A turn that rewrites twenty files is
/// already past the point where a single undo answers the question.
const UNDO_MAX: usize = 20;

/// The pre-image of every change this process made, newest last. `None` means
/// the file did not exist, so putting it back means removing it.
fn undo_stack() -> &'static Mutex<Vec<(PathBuf, Option<String>)>> {
    static STACK: OnceLock<Mutex<Vec<(PathBuf, Option<String>)>>> = OnceLock::new();
    STACK.get_or_init(|| Mutex::new(Vec::new()))
}

/// Remember what `p` held before this change. Best effort: an undo that cannot
/// be recorded must not fail the edit the user asked for.
fn remember(p: &Path, before: Option<String>) {
    let Ok(mut stack) = undo_stack().lock() else {
        return;
    };
    if stack.len() >= UNDO_MAX {
        stack.remove(0);
    }
    stack.push((p.to_path_buf(), before));
}

/// Put the most recent change back and say which file moved.
///
/// This is the half a diff cannot do: the page shows `- old / + new`, and until
/// now the only way back was to edit again by hand.
pub(crate) fn undo_last() -> Result<String, String> {
    let entry = undo_stack().lock().ok().and_then(|mut s| s.pop());
    let Some((path, before)) = entry else {
        return Err("nothing to undo".to_string());
    };
    match before {
        Some(text) => std::fs::write(&path, text)
            .map(|()| format!("restored {}", path.display()))
            .map_err(|e| format!("{}: {e}", path.display())),
        None => {
            // The change created the file, so undoing it removes it.
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(format!("removed {}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok(format!("{} was already gone", path.display()))
                }
                Err(e) => Err(format!("{}: {e}", path.display())),
            }
        }
    }
}

fn read(path: &str, from: Option<u64>, to: Option<u64>) -> Result<String, String> {
    let p = inside(path)?;
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Ok(String::new());
    }
    let first = from.unwrap_or(1).max(1) as usize;
    if first > lines.len() {
        return Err(format!("{} has {} lines", p.display(), lines.len()));
    }
    let last = to
        .map(|n| n as usize)
        .unwrap_or(lines.len())
        .min(lines.len());
    let mut out = String::new();
    for (i, line) in lines[first - 1..last.max(first)].iter().enumerate() {
        let n = first + i;
        out.push_str(&format!("{n:>5}  {line}\n"));
        // shortcut: 100 KB of a file in one reply, enough to read a source file
        // but not a log; range it by line when it is longer.
        if out.len() > 100_000 {
            out.push_str("… truncated, ask for a line range\n");
            break;
        }
    }
    Ok(out)
}

fn write(path: &str, content: &str) -> Result<String, String> {
    let p = inside(path)?;
    if let Some(dir) = p.parent()
        && !dir.is_dir()
    {
        return Err(format!("{} is not a directory", dir.display()));
    }
    remember(&p, std::fs::read_to_string(&p).ok());
    std::fs::write(&p, content).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(format!("wrote {} bytes to {}", content.len(), p.display()))
}

fn list(path: &str) -> Result<String, String> {
    let p = inside(path)?;
    let mut rows = Vec::new();
    for entry in std::fs::read_dir(&p).map_err(|e| format!("{}: {e}", p.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let meta = entry.metadata().ok();
        let dir = meta.as_ref().is_some_and(|m| m.is_dir());
        rows.push(format!(
            "{}{}  {}",
            entry.file_name().to_string_lossy(),
            if dir { "/" } else { "" },
            meta.map(|m| m.len()).unwrap_or(0)
        ));
    }
    rows.sort();
    Ok(rows.join("\n"))
}

fn shell(command: &str, cwd: &str) -> Result<String, String> {
    let dir = inside(cwd)?;
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    let mut child = cmd
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the shell: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if started.elapsed() > SHELL_TIMEOUT => {
                let _ = child.kill();
                return Err(format!(
                    "killed after {}s — run it yourself if it really takes that long",
                    SHELL_TIMEOUT.as_secs()
                ));
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("could not read the output: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        text.push_str("\n[stderr]\n");
        text.push_str(&err);
    }
    let code = out.status.code().unwrap_or(-1);
    if !out.status.success() {
        text.push_str(&format!("\n[exit {code}]"));
    }
    if text.len() > 100_000 {
        text.truncate(100_000);
        text.push_str("\n… truncated");
    }
    Ok(if text.trim().is_empty() {
        format!("(no output, exit {code})")
    } else {
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The terminal registry is one process-wide singleton, so two tests that
    /// both start a shell see each other's — and "no terminal running" would
    /// then be about whichever test happened to be slower.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn a_path_above_the_working_directory_is_refused() {
        for p in ["../secret", "../../etc/passwd", "a/../../out"] {
            let err = inside(p).unwrap_err();
            assert!(err.contains("outside"), "{p}: {err}");
        }
    }

    #[test]
    fn a_path_inside_resolves_under_the_root() {
        let root = root().unwrap();
        let hit = inside("Cargo.toml").unwrap();
        assert_eq!(hit.parent(), Some(root.as_path()));
        assert_eq!(inside(".").unwrap(), root);
    }

    #[test]
    fn every_tool_asks_before_it_runs() {
        let tools = workspace_tools(None);
        assert_eq!(tools.len(), NAMES.len());
        for (t, name) in tools.iter().zip(NAMES) {
            assert_eq!(t.name, name);
            assert!(!t.skip_permission, "{name} must ask");
            assert!(t.handler().is_some(), "{name} has no handler");
            assert!(!t.parameters.is_empty(), "{name} has no schema");
        }
    }

    /// The one tool whose answer is not text: the bytes have to arrive as a
    /// binary result or the model cannot see the picture at all.
    #[test]
    fn view_image_hands_the_bytes_over_instead_of_describing_them() {
        let png = vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3];
        match image_result("shot.png", png.clone(), "image/png") {
            ToolResult::Expanded(e) => {
                let bins = e.binary_results_for_llm.expect("an image result");
                assert_eq!(bins.len(), 1);
                assert_eq!(bins[0].mime_type, "image/png");
                use base64::Engine as _;
                let back = base64::engine::general_purpose::STANDARD
                    .decode(&bins[0].data)
                    .expect("valid base64");
                assert_eq!(back, png);
                assert!(
                    e.text_result_for_llm.contains("shot.png"),
                    "{}",
                    e.text_result_for_llm
                );
            }
            ToolResult::Text(t) => panic!("an image came back as text: {t}"),
            _ => panic!("unexpected tool result shape"),
        }
        // A text file is refused by name rather than shipped as bytes.
        match view("Cargo.toml") {
            ToolResult::Text(t) => assert!(t.contains("not an image"), "{t}"),
            _ => panic!("a .toml was treated as an image"),
        }
        // And the fence still refuses a path outside the working directory.
        match view("../shot.png") {
            ToolResult::Text(t) => assert!(t.contains("outside"), "{t}"),
            _ => panic!("a path above the root was accepted"),
        }
    }

    /// A page with no vendor API behind it still arrives as words: the tag
    /// stripper is the difference between "read this page" and 39 KB of markup.
    #[test]
    fn html_falls_back_to_the_words_not_the_markup() {
        let page = "<html><head><style>.a{color:red}</style><script>var x=1</script></head>\
                    <body><h1>Title &amp; more</h1><p>Hello <b>world</b>.</p></body></html>";
        let text = html_to_text(page);
        assert!(!text.contains('<'), "{text}");
        assert!(!text.contains("var x"), "a script body survived: {text}");
        assert!(!text.contains("color:red"), "a style body survived: {text}");
        assert!(text.contains("Title & more"), "{text}");
        assert!(
            text.contains("Hello world .") || text.contains("Hello world."),
            "{text}"
        );
        // The collapse is what keeps a 40 KB word salad inside the cap.
        assert!(!text.contains("  "), "{text}");
    }

    /// The vendor branch is chosen by the session's provider, offline — a call
    /// with a bad key would reach the network and take a minute to fail.
    #[test]
    fn the_vendor_branch_follows_the_providers_base_url() {
        let ds = Web {
            base: "https://api.deepseek.com".to_string(),
            key: "k".to_string(),
        };
        assert_eq!(ds.deepseek().as_deref(), Some("https://api.deepseek.com"));
        // Trimmed, so the `/v1/tools/...` path joins without a double slash.
        let slashed = Web {
            base: "https://api.deepseek.com/".to_string(),
            key: "k".to_string(),
        };
        assert_eq!(
            slashed.deepseek().as_deref(),
            Some("https://api.deepseek.com")
        );
        // Every other provider gets webrain's engines instead.
        for base in [
            "https://openrouter.ai/api/v1",
            "https://api.openai.com/v1",
            "http://localhost:11434/v1",
        ] {
            assert!(
                Web {
                    base: base.into(),
                    key: "k".into()
                }
                .deepseek()
                .is_none(),
                "{base} was mistaken for deepseek"
            );
        }
    }

    // Read-only: every branch below refuses before the write, so a stray test
    // run cannot leave a modified `Cargo.toml` behind.
    #[test]
    fn an_edit_refuses_an_empty_missing_or_ambiguous_pattern() {
        assert!(
            edit("Cargo.toml", "", "x", false)
                .unwrap_err()
                .contains("empty")
        );
        assert!(
            edit("Cargo.toml", "zzzz-not-in-this-file", "x", false)
                .unwrap_err()
                .contains("no match")
        );
        assert!(
            edit("Cargo.toml", "version", "x", false)
                .unwrap_err()
                .contains("places")
        );
    }

    // The stack is exercised directly rather than through `write`/`edit`: those
    // resolve against the process's working directory, which for this test is
    // the crate root, so a round trip through them would write into the source
    // tree. `remember` is the one call each of them makes.
    #[test]
    fn a_change_can_be_put_back() {
        let dir = std::env::temp_dir().join(format!("webrain-undo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.txt");
        std::fs::write(&file, "edited").unwrap();

        // As `edit` records it: the text before the change.
        remember(&file, Some("original".to_string()));
        assert!(undo_last().unwrap().starts_with("restored"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");

        // As `write` records a file it created: nothing to restore, so undo
        // removes it.
        remember(&file, None);
        assert!(undo_last().unwrap().starts_with("removed"));
        assert!(!file.exists());

        assert!(undo_last().is_err(), "an empty stack has nothing to undo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_delta_shows_what_the_old_text_became() {
        let d = delta("let a = 1;\n", "let a = 2;\n");
        assert!(d.contains("- let a = 1;"), "{d}");
        assert!(d.contains("+ let a = 2;"), "{d}");
        // A long change is cut on both sides rather than filling the row.
        let long = "x\n".repeat(40);
        let d = delta(&long, "y");
        assert_eq!(d.lines().filter(|l| *l == "- x").count(), 12, "{d}");
        // The cut is marked, and the new side still follows it.
        assert!(d.contains("- …"), "{d}");
        assert!(d.ends_with("+ y"), "{d}");
    }

    #[test]
    fn a_search_reports_file_and_line() {
        let hits = search("fn workspace_tools", "src", 5).unwrap();
        assert!(hits.starts_with("src/chat_tools.rs:"), "{hits}");
        assert!(hits.contains("workspace_tools"), "{hits}");
        // Assembled at run time, so this file cannot match its own pattern.
        let missing = ["zzz", "nothing", "here"].join("-");
        assert!(
            search(&missing, "src", 5).unwrap().starts_with("no match"),
            "{missing}"
        );
    }

    #[test]
    fn a_terminal_answers_a_line_and_stops() {
        let _only = one_at_a_time();
        let started = terminal("start", None, None, "", ".").unwrap();
        let id: u64 = started
            .split_whitespace()
            .nth(1)
            .expect("`terminal <id> running`")
            .parse()
            .expect("the second word is the id");
        let said = terminal("send", Some(id), None, "echo webrain-terminal", ".").unwrap();
        let stopped = terminal("stop", Some(id), None, "", ".").unwrap();
        assert!(said.contains("webrain-terminal"), "{said}");
        assert!(stopped.contains("stopped"), "{stopped}");
        // The registry no longer knows it, so an omitted id finds nothing.
        assert!(terminal("read", None, None, "", ".").is_err());
    }

    #[test]
    fn a_job_is_readable_from_both_sides_and_says_when_it_ended() {
        let _only = one_at_a_time();
        // A program that prints and exits: one snapshot then shows both the
        // output and the fact that it is over. `start` takes a program, not a
        // command line, so this is a binary that runs and returns.
        let program = if cfg!(windows) { "where.exe" } else { "true" };
        let started = terminal("start", None, Some(program), "", ".").unwrap();
        let id: u64 = started
            .split_whitespace()
            .nth(1)
            .expect("`terminal <id> running`")
            .parse()
            .expect("the second word is the id");

        // `start` already handed the banner to the model, so this is the page's
        // view of the same bytes: still there, and it did not eat what the model
        // has not read yet — that is the whole point of the cursor.
        let job = wait_until_ended(id);
        assert_eq!(job["command"], program, "{job}");
        assert!(job["exit"].is_number(), "{job}");
        assert!(job["bytes"].as_u64().is_some_and(|b| b > 0), "{job}");

        // The model's read is still "new only", and now it says the job ended
        // rather than leaving a poll against a process that will never print.
        let read = terminal("read", Some(id), None, "", ".").unwrap();
        assert!(read.contains("exited with code"), "{read}");

        // And it can find the job again by name, which is what keeps one started
        // many turns ago usable after the context that named it is gone.
        let listed = terminal("list", None, None, "", ".").unwrap();
        assert!(
            listed.starts_with('#') && listed.contains(program),
            "{listed}"
        );

        // A page-side stop leaves the entry gone, like the tool's own `stop`.
        assert!(job_stop(id).unwrap().starts_with("stopped"));
        assert!(jobs().iter().all(|j| j["id"] != id), "still listed: {id}");
    }

    /// Poll the way the page does: nothing notices a process has exited until
    /// someone asks, so "it ended" is a fact produced by the next snapshot.
    fn wait_until_ended(id: u64) -> Value {
        for _ in 0..60 {
            if let Some(j) = jobs().into_iter().find(|j| j["id"] == id) {
                if j["running"] == false {
                    return j;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("job {id} never ended");
    }
}
