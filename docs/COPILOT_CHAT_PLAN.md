# `webrain chat` — implementation plan (Copilot SDK + BYOK)

Target: one command line.

```bash
webrain chat "scrape news.ycombinator.com and summarise the top 5"
```

Exit 0 on a completed turn, non-zero on failure. Logs on stderr; the answer on
stdout. Decision record: `docs/adr/0003-copilot-sdk-byok-chat.md`.

## What the SDK gives us (verified against the Rust README)

| Need | SDK surface |
|---|---|
| Start the runtime | `Client::start(ClientOptions)`; `client.stop()` flushes telemetry |
| BYOK | `SessionConfig::with_provider(ProviderConfig { provider_type, base_url, api_key/bearer_token, wire_api, model_id, wire_model, headers, max_prompt_tokens, max_output_tokens })` |
| Tools = webrain itself | `with_mcp_servers(IndexMap<String, McpServerConfig>)`, `McpServerConfig::Stdio(McpStdioServerConfig { command, args, env, .. })`; wire names are `<server-key>-<tool-name>` |
| Non-interactive run | `session.send_and_wait(MessageOptions::new(p).with_wait_timeout(d))` |
| Streaming | `config.streaming = Some(true)` + `session.subscribe()`; `assistant.message_delta` |
| Approvals | `PermissionHandler`; built-ins `ApproveAllHandler` / `DenyAllHandler`; `permission::approve_if(..)` |
| Sessions | `resume_session`, `with_infinite_sessions`, `session.workspace_path()` |
| Diagnostics | `client.rpc().models().list()`, `client.startup_timings()` |

Runtime location knobs: `COPILOT_CLI_PATH`, `COPILOT_CLI_EXTRACT_DIR`,
`BUNDLED_CLI_CACHE_DIR`, `COPILOT_SKIP_CLI_DOWNLOAD`.

## CLI surface

```
webrain chat [prompt] [flags]

  --provider openai|anthropic|azure|<key-name>   default: auto-detect from env
  --model ID            provider model id (default per provider table)
  --base-url URL        override the endpoint
  --key-from ENV_NAME   read the key from a different variable
  --engine-provider P   telemetry model_provider (ollama, lm_studio, ...)
  --wire-api completions|responses                  (openai/azure only)
  --header 'K: V'       repeatable; custom provider header
  --max-prompt-tokens N --max-output-tokens N
  --yes                 approve every tool call (non-interactive default is ask)
  --no-tools            run with no MCP servers attached
  --session ID          resume that session instead of creating one
  --cwd DIR             session working directory
  --json                emit {provider, model, answer, events} as JSON
  --print-config        resolve and print provider config (key redacted), then exit
  --stream / --no-stream
```

`--print-config` is the deterministic, offline, testable core of the command:
it exercises the whole resolution chain without spawning the runtime.

## Provider resolution (BYOK)

Order, first match wins. Mirrors `vision_targets_from`
(`webrain-core/src/video.rs:923`) so the two chains cannot drift in style.

1. `--provider` + `--key-from`/`--base-url` flags.
2. `WEBRAIN_CHAT_PROVIDER`, `WEBRAIN_CHAT_MODEL`, `WEBRAIN_CHAT_BASE_URL`,
   `WEBRAIN_CHAT_API_KEY`.
3. Ambient key chain, same aliases as vision:

| Key env (aliases) | `provider_type` | `base_url` | default model |
|---|---|---|---|
| `OPENROUTER_API_KEY` | openai | `https://openrouter.ai/api/v1` | `anthropic/claude-sonnet-4.5` |
| `OPENAI_API_KEY` | openai | `https://api.openai.com/v1` | `gpt-5` |
| `ANTHROPIC_API_KEY` / `CLAUDE_API_KEY` | anthropic | `https://api.anthropic.com/v1` | `claude-sonnet-4-5` |
| `GEMINI_API_KEY` / `GOOGLE_API_KEY` | openai | `https://generativelanguage.googleapis.com/v1beta/openai` | `gemini-2.5-pro` |
| `GROQ_API_KEY` | openai | `https://api.groq.com/openai/v1` | `qwen/qwen3.8-27b` |
| `DASHSCOPE_API_KEY` / `QWEN_API_KEY` | openai | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` | `qwen3-max` |
| `FIREWORKS_API_KEY` | openai | `https://api.fireworks.ai/inference/v1` | `accounts/fireworks/models/qwen3-235b-a22b` |

4. No key → leave `provider` unset so the SDK uses GitHub auth, and print one
   line to stderr naming the variables that would have worked. If the runtime
   is also absent, fail with the install hint rather than a JSON-RPC error.

Model ids live in one const in core and are overridable by `--model`, so a
provider renaming a model is a one-line change (the vision table already
carries that lesson: `qwen/qwen3.6-27b` → `qwen/qwen3.8-27b` on Groq).

## Files and exact touchpoints

| File | Change |
|---|---|
| `webrain-core/src/byok.rs` (new) | `CHAT_PROVIDERS` table, `ChatProvider` struct, `resolve_from(&[(name, value)], flags) -> Option<ChatProvider>`, `redacted()` renderer. No SDK, no network, no env access inside the pure functions. |
| `webrain-core/src/lib.rs` | `pub mod byok;` + re-export (`webrain-core/src/lib.rs`, module list has 16 siblings today). |
| `webrain-cli/Cargo.toml` | `rust-version = "1.94"` override for this package only; `[features] chat = ["dep:github-copilot-sdk"]`; `github-copilot-sdk = { version = "1", optional = true }`; `indexmap` (SDK re-exports `IndexMap`, prefer that). |
| `webrain-cli/src/chat.rs` (new) | `pub async fn run(args: &[String]) -> anyhow::Result<()>`: resolve provider → `ClientOptions` (working_directory, env passthrough) → `Client::start` → `SessionConfig` (provider, mcp_servers, permission handler, streaming, system message) → `send_and_wait` or interactive REPL when no prompt. `#[cfg(feature = "chat")]`. |
| `webrain-cli/src/chat/stub.rs` | The `#[cfg(not(feature = "chat"))]` path: prints the exact rebuild command and exits 2. Release binaries ship the real one. |
| `webrain-cli/src/main.rs:19-152` (`HELP`) | One new row (`"chat"`, usage, about, notes) — the table feeds both `--help` and `webrain chat --help`. |
| `webrain-cli/src/main.rs:160-312` (`ENV`) | New group `Chat (Copilot BYOK)` with `WEBRAIN_CHAT_*`, `COPILOT_CLI_PATH`, `COPILOT_CLI_EXTRACT_DIR`, `BUNDLED_CLI_CACHE_DIR`. `help env` is generated from this table, so omitting it makes the vars undiscoverable. |
| `webrain-cli/src/main.rs:598` dispatch | `Some("chat") => chat::run(&args)?,` before/after `serp`. |
| `webrain-cli/src/main.rs:2150` `run_doctor` | One more line: runtime resolvable? key present? Prints `chat: ready (openai/gpt-5)` / `chat: no key (OPENAI_API_KEY, ANTHROPIC_API_KEY, …)` / `chat: runtime missing`. |
| `.github/workflows/release.yml:103` | Add `chat` to `--features pdfium,chat` for glibc/macOS/Windows targets; keep musl targets `pdfium`-only and have `chat` there print the rebuild hint. |
| `docs/reference/cli.mdx`, `docs/reference/environment.mdx` | New command + variables (the docs are the long form of `HELP`/`ENV`). |
| `docs/COPILOT_CHAT_PLAN.md` (this file) | Add to `docs/.mintignore` (CI `check_docs_orphans.py` fails on an unlisted `.md` under `docs/`). |

`docs/adr/0003-…md` needs no `.mintignore` entry — `adr/` is already listed.

## Phases

**P0 — decision record (done).** `docs/adr/0003-copilot-sdk-byok-chat.md`.

**P1 — pure resolution (no SDK, lands alone).**
`webrain-core/src/byok.rs` + unit tests. Green `cargo test -p webrain-core`
with zero new dependencies. This phase is mergeable even if chat is never built.

**P2 — CLI shape, stubbed runtime.**
`HELP`/`ENV` rows, `Some("chat")` arm, `--print-config`, the not-enabled stub.
`webrain chat --print-config --provider openai --key-from OPENAI_API_KEY` works
in a default build and prints the same JSON the SDK path will use.

**P3 — SDK session.**
`chat.rs` behind `--features chat`; stdio MCP self-attach; permission handler;
streaming; `send_and_wait` with `--json`. Verify by hand on Windows + Linux.

**P4 — doctor, release, docs.**
Doctor line, release matrix, `docs/reference/*`, README quickstart, CHANGELOG.

**P5 (conditional) — licensing fallback.**
If bundling the Copilot CLI is not permissible, switch the feature to
`local-runtime` and add `webrain install copilot` that provisions the runtime
pair into the engine cache (`WEBRAIN_BROWSERS_DIR`-style), with
`COPILOT_CLI_PATH` exported by a wrapper. `webrain chat` becomes a two-step
command; the CLI surface above does not change.

## Permission and safety defaults

- Non-interactive (`--json`, no TTY, or a prompt argument): default is
  `permission::approve_if(|d| !is_destructive(d))` — reads/navigation approved,
  `webrain_batch op=interact` clicks and anything shell-shaped require `--yes`.
- `--yes` installs `ApproveAllHandler` and prints one stderr warning that the
  agent can drive a logged-in browser.
- Chat never reads the vault; credentials still come from `webrain login`.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| MSRV 1.88 → 1.94 | Per-package `rust-version` on `webrain-cli`; `chat` is non-default, so `cargo build --workspace` on 1.88 still works; release jobs pin the newer toolchain. |
| 132–157 MB runtime download per platform at build time | `BUNDLED_CLI_CACHE_DIR` keyed on `<os>-<version>` in CI; documented for offline builds. |
| Linux needs OpenSSL 3 dev files | Release job installs `libssl-dev`; musl targets excluded from `chat`. |
| Binary size | Release notes state the delta; P5 keeps a smaller path available. |
| Copilot CLI redistribution licensing | Resolve before enabling `chat` in `release.yml`; P5 is the answer if it is a no. |
| Windows child-process job objects | Test on `windows-latest`; the SDK already documents its kill-on-close fallback. |
| stdout pollution | `chat` is its own subcommand; MCP stdout contract untouched; logs already go to stderr. |
| Provider/model churn | One const table, `--model` override, `--print-config` makes drift visible in tests. |

## Verification (quantified objective)

Objective: **`webrain chat --print-config` resolves all 7 provider rows from a
synthetic environment, and one live turn completes with webrain's own MCP tools
attached, exit 0, in under 120 s.**

1. **Table test (no network, P1).** Feed `resolve_from` a synthetic
   `[(name, value)]` slice (same style as `vision_targets_from`) with one row
   per provider plus the alias pairs (`ANTHROPIC_API_KEY`/`CLAUDE_API_KEY`,
   `GEMINI_API_KEY`/`GOOGLE_API_KEY`, `DASHSCOPE_API_KEY`/`QWEN_API_KEY`).
   Assert exact `provider_type` / `base_url` / `model` per row, first-match
   ordering, and that an unknown provider errors instead of silently defaulting.
   Metric: 7/7 rows + 3 alias cases green.
2. **Config test (no network, P2).** Golden-file test on `--print-config`
   output for all 7 rows, asserting the key is redacted and that no key bytes
   appear anywhere in stdout/stderr.
3. **Fake-provider test (loopback, P3).** A test-only listener on
   `127.0.0.1:0` that answers `/v1/chat/completions` with a canned completion.
   `webrain chat "hi" --provider openai --base-url http://127.0.0.1:PORT/v1
   --key-from WEBRAIN_TEST_KEY --json` must (a) exit 0, (b) emit `answer` equal
   to the canned text, (c) show the request hit the expected path. Gated behind
   `--features chat` and `#[ignore]` so CI stays offline.
4. **Tool-attachment assertion (P3).** With the fake provider echoing the
   request, assert the outgoing payload lists the `webrain` MCP server and that
   `session.rpc().models().list()` / tool listing includes a
   `webrain-webrain_*` name — i.e. the agent really sees webrain's tools.
5. **Doctor matrix (P4).** Three states (`ready`, `no key`, `runtime missing`)
   asserted by a unit test over an injected env + path probe.
6. **Manual e2e, documented in the CHANGELOG.** `webrain chat "list the files in
   docs and name the one that defines the MCP nav" --yes` returns `docs.json`.

Rerun 1–2 and 5 on every change; 3–4 when the `chat` feature is touched; 6 at
release.
