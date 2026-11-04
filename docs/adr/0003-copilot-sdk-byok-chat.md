# ADR-003: `webrain chat` — GitHub Copilot SDK with BYOK

- Status: Accepted (implemented 2026-11-04)
- Date: 2026-11-04
- Supersedes: none
- Related: ADR-001 (architecture), ADR-002 (intent tool surface)

## Context

webrain ships one binary that already contains everything an agent needs:
18 MCP tools (`webrain_*`), three CDP engines, a vault, SERP tiers, and a
vision/BYOK key chain (`webrain-core/src/video.rs:863` `VISION_PROVIDERS`,
`vision_env_keys` at 906, `vision_targets_from` at 923). What it does not ship
is a *driver*: today a user must install Copilot CLI, Claude Code, Codex or
Cursor and point it at `webrain mcp`.

`VISION_PROVIDERS` already proves the repo's convention for bring-your-own-key:
a table of `(env-name aliases, endpoint, model)` rows, resolved in order, first
non-empty key wins. Chat has the same shape but a different model set.

GitHub publishes a Rust SDK for the Copilot agent loop
(`github-copilot-sdk`, https://github.com/github/copilot-sdk/blob/main/rust/README.md)
that gives us, without writing an agent loop ourselves:

- `Client::start(ClientOptions)` — spawns/reuses the Copilot CLI runtime over
  JSON-RPC (stdio), health-checks it, reaps it.
- `ProviderConfig` — BYOK. `provider_type` (`openai` | `azure` | `anthropic`),
  `base_url`, `api_key` / `bearer_token`, `wire_api`
  (`completions` | `responses`), `model_id`, `wire_model`, `headers`,
  `max_prompt_tokens`, `max_output_tokens`. Provider types include
  OpenAI-compatible gateways, so OpenRouter/Groq/DashScope work as `openai`.
- `SessionConfig::with_mcp_servers(IndexMap<String, McpServerConfig>)` — attach
  MCP servers. Wire tool names become `<server-key>-<tool-name>`.
- `PermissionHandler` / `SessionHooks` / `ToolHandler` / streaming
  (`session.subscribe()`, `assistant.message_delta`), `send_and_wait`,
  `AutoTier`, infinite sessions, OpenTelemetry export.

So `webrain chat "…"` can be one command that runs a Copilot session whose model
traffic goes to the user's own key and whose tools *are webrain's own MCP
server* — a self-driving webrain.

## Decision

Add one subcommand, `webrain chat`, that:

1. Resolves a BYOK provider from flags → `WEBRAIN_CHAT_*` env → the ambient key
   chain (same aliases as vision) → falls back to the SDK's GitHub-auth path,
   and errors with an actionable message when none exists.
2. Starts the Copilot SDK client and a session with
   `provider = Some(ProviderConfig)`.
3. Attaches `webrain mcp` (same executable, `std::env::current_exe()`, stdio) as
   an MCP server named `webrain`, so the agent gets `webrain-webrain_*` tools.
4. Sends the prompt with `send_and_wait`, streams deltas to stdout, and exits
   non-zero on session error.
5. Is feature-gated as `chat` in `webrain-cli` (not core), because the SDK
   brings a downloaded/bundled CLI runtime and a 1.94 MSRV. Core stays
   dependency-light; the provider *resolution* logic lives in core as pure,
   unit-testable data.

`webrain chat` is additive. It never changes the MCP command's stdout contract,
the existing key chain, or the default build.

## Consequences

### Positive

- One command, zero external agent install: `webrain chat "scrape X and summarise"`.
- BYOK reuses the key names users already set for vision, so no second config.
- The tool surface, permission model, streaming and session persistence come
  from a Microsoft-maintained SDK instead of a hand-rolled loop.
- Provider resolution is pure data in core → testable without the SDK, the
  network, or a key.

### Negative

- `chat` builds pull `github-copilot-sdk` with `bundled-cli` by default:
  ~132–157 MB of runtime per platform, downloaded and SHA-verified at build
  time (`BUNDLED_CLI_CACHE_DIR` caches it). The binary grows accordingly.
- The crate requires Rust 1.94 and, on Linux, OS OpenSSL 3 dev files for the
  default `network-forwarding` feature. Because the workspace uses resolver 3
  (MSRV-aware), the floor applies to *every* build, not just `--features chat`
  — so the workspace `rust-version` moved to 1.94 rather than being overridden
  per package.
- The two static musl release targets cannot carry `chat`: the SDK does publish
  `linuxmusl-*` runtime assets, but `network-forwarding` links the OS OpenSSL 3
  and the SDK rejects a static or vendored copy.
- Bundling the Copilot CLI into a redistributed binary is a licensing question.
  The release matrix now builds `--features pdfium,chat` for the six non-musl
  targets, so **that answer is now a release blocker rather than a design
  question**: if redistribution is not permitted, revert the matrix to
  `--features pdfium` and keep `chat` source-only — the code does not change.

### Neutral

- Chat is a separate arm of the existing manual `match` dispatch; no clap, no
  new parser, consistent with `webrain serp`/`webrain diff`.
- The same `mcp` server is spawned as a child of `chat`; no port, no token.

## Alternatives considered

- **Hand-rolled OpenAI-compatible tool loop (~300 lines, no SDK).** Smallest
  binary, works on musl, no MSRV bump — but re-implements permissions, hooks,
  streaming, session resumption and MCP client plumbing, and diverges from the
  agent behaviour users get elsewhere. Keep as the fallback if licensing or
  musl support blocks the SDK.
- **`local-runtime` + `webrain install copilot`** (download the runtime into the
  engine cache, mirroring `webrain install --engine chrome`). Preserves the
  current binary size and the repo's "install the engine" pattern, but the
  produced binary must resolve `COPILOT_CLI_PATH` at runtime, so `webrain chat`
  is a two-step command. Deferred, not rejected: it is the shipping path if the
  licensing answer is no.
- **Wire chat into `webrain mcp` as a tool (`webrain_agent`).** Rejected:
  recursive agent-inside-agent with a shared stdout, and it breaks the MCP
  purity rule.

## Graph evidence

Measured 2026-11-04 from the codebase-memory graph
(`D-Windows-Documents-Programming-Projects-Rust-webrain`, 2 870 nodes / 7 314
edges):

- Entry point: `webrain-cli/src/main.rs:541` `main` (1 124 lines), dispatch
  `match args.get(1)`; help lives in one table `main.HELP` (19-152) and env docs
  in `main.ENV` (160-312) rendered by `env_section` (315-324) / `help_page`
  (473-503) — so a new subcommand is one row plus one arm.
- Existing BYOK precedent: `webrain-core/src/video.rs:863` `VISION_PROVIDERS`
  (7 rows), `vision_env_keys` (906), `vision_targets_from` (923) — the shape to
  copy, including the keys-slice parameter that makes it testable.
- MCP surface: `webrain-mcp/src/tools.rs` `list_tools` (688-990),
  `webrain-mcp/src/lib.rs` `run_stdio` (1399-1429), `handle_rpc` (183-1201),
  `tool_names` (23-28).
- Doctor: `main.run_doctor` (2150-2227) already reports engines/CDP/MCP/vault;
  chat runtime + key presence belong there.
- CI: `.github/workflows/ci.yml` runs `fmt` + `clippy --workspace --all-targets
  --features pdfium` + `test`; `release.yml:103` builds with `--features pdfium`
  per target, including musl variants.
- Core module set (`webrain-core/src/*.rs`): nothing chat- or LLM-loop-shaped
  today; vision and video are the only files holding provider tables.

## Implementation notes (2026-11-04)

What landed, and where it differs from the plan above:

| File | Role |
|---|---|
| `webrain-core/src/byok.rs` | `CHAT_PROVIDERS` (7 rows), `resolve_from`, `ChatProvider` (private `bearer`, redacting `Debug`), `CHAT_ENV_GROUP` — spliced into the CLI's `ENV` table so `help env` cannot advertise a variable the resolver does not read |
| `webrain-cli/src/chat.rs` | flag parsing (no clap), `--print-config`, the no-feature stub |
| `webrain-cli/src/chat_session.rs` | the SDK session, `#[cfg(feature = "chat")]` only |

Deviations worth recording:

- `ProviderConfig` carries **both** `api_key` and `bearer_token` (bearer wins);
  the plan listed only one. The implementation sets `api_key`, which is what the
  OpenAI-compatible rows expect.
- The model is set in two places on purpose: `SessionConfig::with_model` (what
  the runtime selects) and `ProviderConfig::model_id` (what the provider is told
  to bill/serve). They are the same string, but the runtime falls back to the
  session value when the provider leaves it unset.
- A named provider uses **only its own** variables. An earlier draft borrowed any
  ambient key, which would have shipped an OpenAI key to Groq; a test now pins
  that it errors instead.

Verification performed:

- `cargo test -p webrain-core byok` — 12 tests (7 rows, 3 alias pairs, order,
  unknown provider, `--key-from` unset, keyless base url, redaction, `WEBRAIN_CHAT_*`).
- `cargo test -p webrain-cli chat` — 5 tests over flag parsing and the redacted
  config payload.
- clippy clean in both feature states; `mintlify validate` and the docs orphan
  check pass; `webrain doctor` prints the three chat states.

**Open item:** a live turn against Groq's `qwen/qwen3.8-27b` returned the SDK's
`No response was returned.`, i.e. the session was created and the MCP server was
configured, but the provider produced no usable answer (that model may not
support tool calling). `WEBRAIN_CHAT_DEBUG=1` now mirrors every raw event so the
provider's own error is visible.

## Follow-up from live traces (2026-11-04, second pass)

The first live attempts failed for three reasons that the event stream named, and
that no amount of reading the API docs had surfaced:

| Symptom | Cause | Fix |
|---|---|---|
| `compaction_static_context_blocked`, then `session.error {"errorType":"query"}` | the runtime read this machine's `~/.copilot` and merged ~20 MCP servers; `toolDefinitionsTokens: 137019` vs `tokenLimit: 128000` | `ClientMode::Empty` + `base_directory` (`WEBRAIN_CHAT_HOME`) — Empty mode *requires* an explicit state dir and refuses the implicit fallback |
| 8048 static tokens per turn | inheriting the runtime's coding-agent system prompt | `SystemMessageConfig` `mode: "replace"` with a webrain prompt → **272** |
| `402 ... requested up to 64000 tokens, but can only afford 171` | we let the runtime ask for a 64k completion, which some providers reserve against the balance | `max_output_tokens` default **4096** |

Also adopted from the upstream README: `ToolSet` for `available_tools`
(`mcp:webrain-<tool>`), `MemoryConfiguration::disabled()`, and
`enable_session_telemetry = false`. Not adopted: `disable_client_telemetry` and
`install_bundled_runtime`, which are newer than the pinned `1.0.28`.

**What is still unproven, and why it is not a code defect:** a completed tool
turn. A tool-enabled prompt is ~8.4k tokens by the runtime's own count (18
`webrain_*` schemas ≈ 8080 + 272 system), the allowlist does not reduce that
count (only what the model may call), and the runtime's builtin
`github-mcp-server` cannot be unloaded from `ClientOptions`. Groq's free tier
allows 7000 input tokens/minute — `413 ... Requested 10020 ... Limit 7000` — and
the second key on hand can afford 171 tokens. The guide and the CLI's error path
both explain this; proving the turn needs a key with headroom, or a deliberate
decision to expose fewer than 18 tools.
