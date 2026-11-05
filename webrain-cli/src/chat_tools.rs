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

use github_copilot_sdk::Error;
use github_copilot_sdk::ToolResult;
use github_copilot_sdk::tool::ToolHandler;
use github_copilot_sdk::types::{Tool, ToolInvocation};
use serde_json::{Value, json};

/// A shell command that has not answered by now is a command the user did not
/// mean to run. It is killed, and the model is told so.
const SHELL_TIMEOUT: Duration = Duration::from_secs(120);

/// The names the session's tool allowlist has to carry, in the order the prompt
/// lists them. A box in the page's tools dialog is checked for each of these.
pub(crate) const NAMES: [&str; 7] = [
    "read_file",
    "write_file",
    "list_dir",
    "edit_file",
    "search_files",
    "run_shell",
    "terminal",
];

/// The seven tools, with their schemas and their handlers attached.
pub(crate) fn workspace_tools() -> Vec<Tool> {
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
            "A shell that stays alive between calls. `start` spawns it (the \
             platform shell unless `command` says otherwise) and `send` types a \
             line and returns what it printed — so a prompt, a REPL or a dev \
             server is answerable. `read` returns only the output since the last \
             call; `stop` kills it. Omit `id` to mean the newest terminal.",
            json!({
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": ["start", "send", "read", "stop"],
                           "description": "start | send a line and read the reply | read new output | kill it"},
                    "id": {"type": "integer", "description": "terminal id from `start`; omit for the newest"},
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
    let root = root()?;
    let want = Path::new(if p.trim().is_empty() { "." } else { p });
    let joined = if want.is_absolute() {
        want.to_path_buf()
    } else {
        root.join(want)
    };
    let norm = lexically(&joined);
    if !norm.starts_with(&root) {
        return Err(format!("refusing {p}: outside {}", root.display()));
    }
    match std::fs::canonicalize(&norm) {
        Ok(real) if real.starts_with(&root) => Ok(real),
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
    std::fs::write(&p, text.replacen(old, new, n)).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(format!(
        "edited {}: {n} replacement{}",
        p.display(),
        if n == 1 { "" } else { "s" }
    ))
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

/// A shell that outlives the call that started it. The model's handle is the
/// number; the process lives here.
struct Term {
    child: Child,
    stdin: ChildStdin,
    out: Arc<Mutex<String>>,
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
            let out = {
                let s = sessions().lock().unwrap();
                s.all[&which(&s, id)?].out.clone()
            };
            Ok(taken(&out))
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
        other => Err(format!("unknown op `{other}`: start | send | read | stop")),
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
    let out = Arc::new(Mutex::new(String::new()));
    if let Some(pipe) = child.stdout.take() {
        drain(pipe, out.clone());
    }
    if let Some(pipe) = child.stderr.take() {
        drain(pipe, out.clone());
    }
    let mut s = sessions().lock().unwrap();
    s.next += 1;
    let id = s.next;
    s.all.insert(
        id,
        Term {
            child,
            stdin,
            out: out.clone(),
        },
    );
    drop(s);
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
fn drain<R: Read + Send + 'static>(mut pipe: R, out: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
            let mut text = out.lock().unwrap();
            text.push_str(&String::from_utf8_lossy(&buf[..n]));
            // shortcut: the last 100 KB of a chatty process, not the whole log;
            // a longer tail is heap nobody reads.
            if text.len() > 100_000 {
                let cut = text.len() - 100_000;
                let cut = (cut..text.len())
                    .find(|i| text.is_char_boundary(*i))
                    .unwrap_or(text.len());
                text.drain(..cut);
            }
        }
    });
}

/// New output only: a terminal that replayed its scrollback on every call would
/// cost more context than the tool saves.
fn taken(out: &Arc<Mutex<String>>) -> String {
    let text = std::mem::take(&mut *out.lock().unwrap());
    if text.is_empty() {
        "(no new output)".to_string()
    } else {
        text
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
        let tools = workspace_tools();
        assert_eq!(tools.len(), NAMES.len());
        for (t, name) in tools.iter().zip(NAMES) {
            assert_eq!(t.name, name);
            assert!(!t.skip_permission, "{name} must ask");
            assert!(t.handler().is_some(), "{name} has no handler");
            assert!(!t.parameters.is_empty(), "{name} has no schema");
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
}
