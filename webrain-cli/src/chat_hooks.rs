//! The fence `webrain chat` puts in front of the runtime's OWN tools.
//!
//! The seven tools in `chat_tools` resolve every path against the working
//! directory. The runtime's built-ins do not: `tool_filter` admits them with
//! `builtin:*`, and the only thing between them and the filesystem is a
//! permission prompt — an approval, not a boundary. A user who clicks "allow"
//! approves a `bash` that runs anywhere the process can.
//!
//! `preToolUse` is the one point the runtime consults before dispatch, so the
//! boundary lives there and holds whatever the user answers.
//!
//! The same hook injects the workspace's own instructions at session start, so a
//! `webrain chat` started in a repository follows that repository's `AGENTS.md`
//! the way every other agent in it does.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use copilot_sdk::hooks::{
    HookContext, PreToolUseInput, PreToolUseOutput, SessionHooks, SessionStartInput,
    SessionStartOutput, SubagentStartInput, SubagentStartOutput,
};
use serde_json::Value;

/// The runtime's shell: no path argument to inspect, and the process's whole
/// filesystem as its reach. Denied by name — the fenced equivalents exist.
const BUILTIN_SHELL: [&str; 2] = ["bash", "shell"];

/// Built-ins that take a path and would otherwise reach outside the directory.
const BUILTIN_PATH_TOOLS: [&str; 4] = ["str_replace_editor", "edit", "write", "view"];

/// Argument names the path-taking built-ins use across runtime versions.
const PATH_ARGS: [&str; 5] = ["path", "file_path", "filePath", "filename", "file"];

/// Files an agent working in this directory is expected to obey, most specific
/// first. The first one that exists wins the whole budget.
const NOTES: [&str; 2] = ["AGENTS.md", ".github/copilot-instructions.md"];

/// Prompt budget for workspace instructions. Every byte here is a byte off the
/// per-turn prompt (`systemTokens` + `toolDefinitionsTokens` is already ~8.4k),
/// so the file is truncated rather than trusted.
const NOTES_CAP: usize = 4096;

/// The working directory this chat may touch, captured once at connect: a `cd`
/// inside a built-in shell must not move the boundary.
pub(crate) struct Fence {
    root: PathBuf,
}

impl Fence {
    pub(crate) fn new(root: PathBuf) -> Self {
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        Self { root }
    }
}

#[async_trait]
impl SessionHooks for Fence {
    async fn on_pre_tool_use(
        &self,
        input: PreToolUseInput,
        _ctx: HookContext,
    ) -> Option<PreToolUseOutput> {
        // `None` = no hook result, the CLI runs the call unchanged.
        let reason = refuse(&input.tool_name, &input.tool_args, &self.root)?;
        Some(PreToolUseOutput {
            permission_decision: Some("deny".to_string()),
            permission_decision_reason: Some(reason),
            ..Default::default()
        })
    }

    async fn on_session_start(
        &self,
        input: SessionStartInput,
        _ctx: HookContext,
    ) -> Option<SessionStartOutput> {
        Some(SessionStartOutput {
            additional_context: workspace_notes(&input.working_directory),
            ..Default::default()
        })
    }

    /// A sub-agent starts with an empty prompt, so `on_session_start` never
    /// reaches it: without this a delegated task knows less about the workspace
    /// than the turn that spawned it. The tool gate still holds — `preToolUse`
    /// fires for sub-agent calls too — but the rules travel with the context.
    async fn on_subagent_start(
        &self,
        input: SubagentStartInput,
        _ctx: HookContext,
    ) -> Option<SubagentStartOutput> {
        Some(SubagentStartOutput {
            additional_context: workspace_notes(&input.working_directory),
        })
    }
}

/// Why this call is refused, or `None` when it may run.
///
/// Only the runtime's own tools are policed. The `webrain_*` server tools touch
/// no local path, and the `chat_tools` ones already resolve through the same
/// boundary — denying them here would be the second copy of one rule.
fn refuse(tool: &str, args: &Value, root: &Path) -> Option<String> {
    // The allowlist spells built-ins `builtin:<name>`; the hook may see either.
    let name = tool.strip_prefix("builtin:").unwrap_or(tool);
    if BUILTIN_SHELL.contains(&name) {
        return Some(format!(
            "`{name}` is disabled in `webrain chat`: it runs outside {} and skips the \
             fence. Use `run_shell` for one command, or `terminal` for a shell that \
             stays open — both stay inside the directory and ask before they run.",
            root.display()
        ));
    }
    if !BUILTIN_PATH_TOOLS.contains(&name) {
        return None;
    }
    let want = PATH_ARGS
        .iter()
        .find_map(|k| args.get(*k).and_then(Value::as_str))?;
    (!crate::chat_tools::within(root, want)).then(|| {
        format!(
            "refusing `{name}` on {want}: outside {} — use `read_file`/`write_file`/\
             `edit_file`, which are fenced to this directory",
            root.display()
        )
    })
}

/// The workspace's own instructions, if it has any, as startup context.
///
/// `None` when neither file exists reads the same to the model as an empty one,
/// so the caller never has to care which.
fn workspace_notes(dir: &Path) -> Option<String> {
    let mut budget = NOTES_CAP;
    let mut out = String::new();
    for rel in NOTES {
        if budget == 0 {
            break;
        }
        let Ok(text) = std::fs::read_to_string(dir.join(rel)) else {
            continue;
        };
        let text = truncate_chars(text.trim(), budget);
        if text.is_empty() {
            continue;
        }
        budget = budget.saturating_sub(text.len());
        out.push_str(&format!("\n\n### {rel}\n{text}"));
    }
    (!out.is_empty()).then(|| {
        format!(
            "The directory you are working in carries its own instructions. Follow them; \
             they take precedence over the defaults above.{out}"
        )
    })
}

/// `text` cut to at most `cap` bytes without splitting a character.
fn truncate_chars(text: &str, cap: usize) -> &str {
    if text.len() <= cap {
        return text;
    }
    let mut end = cap;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("webrain-fence-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_builtin_shell_is_refused_and_the_fenced_one_named() {
        let reason = refuse("bash", &json!({}), &root()).expect("bash is denied");
        assert!(reason.contains("run_shell"), "{reason}");
        assert!(reason.contains("terminal"), "{reason}");
        // The allowlist may hand us either spelling.
        assert!(refuse("builtin:bash", &json!({}), &root()).is_some());
    }

    #[test]
    fn our_own_tools_and_the_scraping_server_pass_through() {
        for tool in [
            "read_file",
            "write_file",
            "edit_file",
            "run_shell",
            "terminal",
            "webrain-webrain_navigate",
        ] {
            assert!(
                refuse(tool, &json!({ "path": "../up" }), &root()).is_none(),
                "{tool} must not be policed twice"
            );
        }
    }

    #[test]
    fn a_builtin_path_tool_outside_the_directory_is_refused() {
        let root = root();
        let outside = json!({ "path": "../../etc/hosts" });
        let reason = refuse("str_replace_editor", &outside, &root).expect("denied");
        assert!(reason.contains("outside"), "{reason}");

        // An absolute path outside is the same decision, on every platform.
        let sibling = root
            .parent()
            .unwrap_or(&root)
            .join("definitely-outside.txt");
        assert!(refuse("view", &json!({ "path": sibling }), &root).is_some());
        // Inside is allowed, in both spellings of the argument.
        assert!(refuse("view", &json!({ "path": "src/lib.rs" }), &root).is_none());
        assert!(refuse("edit", &json!({ "filePath": "a/b.txt" }), &root).is_none());
    }

    #[test]
    fn workspace_notes_carry_the_first_file_and_stop_at_the_budget() {
        // A unique leaf: `root()` is keyed on the pid, and Windows reuses pids, so
        // a previous run's AGENTS.md turned "empty dir" into a false failure.
        let dir = root().join(format!(
            "notes-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(dir.join(".github")).unwrap();
        assert!(workspace_notes(&dir).is_none(), "empty dir -> no context");

        std::fs::write(dir.join("AGENTS.md"), "answer in French").unwrap();
        let notes = workspace_notes(&dir).expect("AGENTS.md is picked up");
        assert!(notes.contains("answer in French"), "{notes}");
        assert!(notes.contains("AGENTS.md"), "{notes}");

        // A file over the budget is cut, not dropped.
        std::fs::write(dir.join("AGENTS.md"), "x".repeat(NOTES_CAP * 2)).unwrap();
        let notes = workspace_notes(&dir).expect("a long file is still context");
        assert!(notes.len() < NOTES_CAP + 400, "capped: {}", notes.len());
    }

    #[test]
    fn truncation_never_splits_a_character() {
        let text = "αααα"; // 2 bytes each
        assert_eq!(truncate_chars(text, 5), "αα");
        assert_eq!(truncate_chars(text, 100), text);
    }
}
