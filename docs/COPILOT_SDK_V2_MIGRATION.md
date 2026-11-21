# Migrating `webrain chat` from `github-copilot-sdk` 1.0.28 to `copilot-sdk` 2.0.1

Measured 2026-11-21 by downloading both crates and reading them, not from release
notes. Every number below is a count from that source.

## This is a crate rename, not a version bump

`github-copilot-sdk` is deprecated. Its own 1.0.35 metadata reads:

> Deprecated GitHub Copilot SDK. Migrate to copilot-sdk.

crates.io, 2026-11-21:

| crate | max stable | newest | note |
|---|---|---|---|
| `github-copilot-sdk` | 1.0.35 | 1.0.34-unstable.44588354783.g40dd772 | deprecated, 1.0.x only |
| `copilot-sdk` | **2.0.1** | 2.0.2-preview.4 | the successor; default feature `bundled-in-process` |

So "there is no 2.0" was true of the old name and false of the real one. The pin
in `webrain-cli/Cargo.toml` moved from `github-copilot-sdk = "1"` to
`copilot-sdk = "2.0.1"`.

Feature names we use survived unchanged: `runtime`, `network-forwarding`,
`bundled-cli` (plus `in-process`, `local-runtime`, `bundled-in-process`).
`COPILOT_SKIP_CLI_DOWNLOAD=1` is a hard opt-out of the build-time runtime
download, so a compile-only check costs nothing.

## What the migration costs

`SessionConfig` — 99 flat fields, the thing `connect()` builds — **does not exist
in 2.0.1**. It is replaced by `CreateSessionOptions` with **31 fields**, most of
them group structs. That regrouping is the whole job; everything else is small.

| surface | 1.0.28 | 2.0.1 | verdict |
|---|---|---|---|
| crate path in source | `github_copilot_sdk` (22 sites, 5 files) | `copilot_sdk` | mechanical, **done** |
| session config | `SessionConfig`, 99 flat fields | `CreateSessionOptions`, 31 fields (19 groups) | **the work** |
| `PermissionHandler::handle` | `(&self, session, request_id, data)` | identical | unchanged |
| `UserInputHandler::handle` | 5 params | 6 — gains `request_id: Option<RequestId>` | one signature |
| `ToolHandler::call` | `(&self, ToolInvocation) -> Result<ToolResult, Error>` | identical | unchanged |
| modules | `types`, `rpc`, `session_events`, `handler`, `hooks`, `tool`, `session` | same names, `rpc` is now a facade over a private `generated` module | imports still resolve |
| `ClientOptions` | `default()` + fields | `new(mode)`; `working_directory`/`program` moved into `RuntimeConnection::SpawnedStdio(SpawnedProcessOptions)` | one block |
| session create | `client.create_session(SessionConfig)` | `client.create_session(CreateSessionOptions)` | same call |
| 53 type names we import | — | **29 changed or gone** | compiler-listed |

## Where each setting goes

Our 21 settings, mapped to the group that now holds them. No field was dropped
except `enable_session_telemetry`, which no longer exists (we set it to `false`).

| today (flat) | 2.0.1 |
|---|---|
| `name` | `CreateSessionOptions.name` |
| `with_model` / `with_provider(ProviderConfig)` | `inference.providers: Vec<NamedProviderConfig>` + `inference.default_provider` + `inference.models` |
| `with_system_message(SystemMessageConfig)` | `system_message` (unchanged type) |
| `with_infinite_sessions` | `state.infinite_sessions` |
| `with_permission_handler` / `approve_all_permissions` / `approve_permissions_if` | `permissions.on_request`, `permissions.mode` |
| `with_user_input_handler` | `interaction.on_user_input_request` |
| `with_hooks` | `hooks.handlers`, `hooks.has_handlers` |
| `enable_skills` + `with_skill_directories` | `skills.enabled`, `skills.directories` |
| `streaming` | `events.streaming` |
| `reasoning_effort`, `reasoning_summary` | `inference.reasoning_effort`, `inference.reasoning_summary` |
| `include_sub_agent_streaming_events` | `events.include_sub_agent_streaming_events` |
| `enable_file_change_tracking` | `filesystem.track_file_changes` |
| `enable_session_store` | `state.enable_cross_session_store` (renamed) |
| `memory` | `state.memory` |
| `enable_config_discovery` | top-level, unchanged |
| `available_tools`, `excluded_tools` | `tools.available_tools`, `tools.excluded_tools` |
| `tool_search` | `tools.search` |
| `large_output` | `tools.large_output` |
| `organization_custom_instructions` | `instructions.organization_text` |
| `with_mcp_servers` | `mcp.servers` |
| `working_directory` | top-level, unchanged |
| `enable_session_telemetry` | **gone** — delete the line |

## The rest of the compiler's list

First `cargo check` against 2.0.1 — after the crate rename — reported **8
errors**, which is the whole call-site surface:

1. `SessionConfig` import (chat_session.rs) — the rewrite above.
2. `UserInputHandler::handle` needs `request_id` (chat_server.rs).
3. `SessionUpdateOptionsParams` lost `organization_custom_instructions`,
   `enable_session_store`, `reasoning_effort` — it is regrouped too; it now
   offers `agents`, `client_name`, `events`, `execution`, `experiments` and 19
   more.
4. `McpEnableRequest` / `McpDisableRequest` gained `expected_installation_id`.
5. `ClientOptions::default()` → `ClientOptions::new(mode)`.

## The plan

Ordered so each step is verifiable before the next depends on it. Sizes are the
work in `connect()` (M) or a route plus a card (S/M).

### Step 1 — make it compile: `connect()` (the gate)

Nothing else is reachable until this lands. The two open errors are the outline;
the group measurements below are what makes it mechanical.

**1a. `ClientOptions`** — `with_base_directory` and `with_mode` still exist, so
only the program moves:

```rust
let mut options = ClientOptions::new(ClientMode::Empty).with_base_directory(home);
if let Some(bin) = webrain_core::install::find_copilot() {
    options.child_process_options_mut().program = ...;   // was with_program(CliProgram::Path)
}
```

(`for_spawned_stdio` is the other door; `child_process_options_mut` keeps the
existing shape.)

**1b. The 12 settings, by group** — `SessionMcpOptions { servers }`,
`SessionToolOptions { available_tools, excluded_tools, search, large_output }`,
`SessionEventOptions { streaming, include_sub_agent_streaming_events }`,
`SessionStateOptions { infinite_sessions, memory, enable_cross_session_store }`,
`SessionSkillOptions { enabled, directories }`,
`SessionInstructionOptions { organization_text }`,
`SessionFilesystemOptions { track_file_changes }`,
`SessionHookOptions { handlers }`,
`SessionInteractionOptions { on_user_input_request }`.

**1c. The provider is three pieces now, and two of them hold the cost fix.**
`ProviderConfig` is gone; `NamedProviderConfig` has no `model_id`,
`max_prompt_tokens` or `max_output_tokens` — those live on `ProviderModelConfig`,
which is what `inference.models` takes:

```rust
inference: Some(SessionInferenceOptions {
    providers: vec![NamedProviderConfig {
        name: "webrain".into(),
        base_url: provider.base_url.clone(),
        provider_type: provider.provider_type.clone(),
        api_key: provider.bearer_token(),
        ..Default::default()
    }],
    default_provider: Some(Some("webrain".into())),
    models: vec![ProviderModelConfig {
        id: provider.model.clone(), provider: "webrain".into(),
        model_id: Some(provider.model.clone()),
        max_prompt_tokens: args.max_prompt_tokens,
        max_output_tokens: Some(i64::from(args.max_output_tokens.unwrap_or(4096))),
        ..Default::default()
    }],
    ..Default::default()
})
```

Dropping `max_output_tokens` here silently re-breaks what the comment in
`connect()` documents: OpenRouter refusing a request that reserves 64k
("you requested up to 64000 tokens, but can only afford 171").

**1d. Permissions — the one silent failure in the step.**
`SessionPermissionOptions { sandbox, mode: Option<PermissionMode>, on_request:
Option<Arc<dyn PermissionHandler>> }`, and `PermissionMode` is
`Manual | Assisted | AllowAll | Unknown`. `approve_all_permissions` →
`mode: Some(AllowAll)`; our handler → `on_request: Some(handler)`.
`approve_permissions_if(not_destructive)` has **no** 2.0.1 equivalent — it was a
predicate on the default flow, so it is answered either by our own handler or by
`Assisted`. If neither takes it, the consequence is not a compile error: every
tool call reaches the page as an unanswered prompt.

**1e.** Delete `enable_session_telemetry` (gone).

Check: `cargo check -p webrain-cli --features pdfium,chat` exit 0, then
`cargo test -p webrain-cli --features pdfium,chat` = the 58 that pass today.

### Step 2 — parity, not just compilation

Reused because it is cheap and it is the only thing that catches 1d: one live
`webrain chat --serve` turn that calls a tool (approval card appears and is
answerable), one `POST /settings` (patching still answers `restart:false`), one
answer that streams. A migration that compiles and never asks for permission is
worse than one that does not compile.

### Step 3 — the three capabilities that change behaviour

| # | setting | change | check |
|---|---|---|---|
| 3.1 | `inference.allow_multiple_tool_calls` | **SHIPPED** as the `parallel_tools` switch | two tool rows before the answer on a prompt naming two files |
| 3.2 | `execution.limits` | **NOT A FEATURE — measured.** `SessionLimitsConfig` has one field, `max_ai_credits`, which is a Copilot-plan credit budget, not a cap on a runaway turn. Nothing in 2.0.1 stops a turn except our own `timeout_secs`, which already exists | — |
| 3.3 | `interaction.on_elicitation_request` + `ask_user_variant` | **L, not M.** `ElicitationRequest` carries `requested_schema: Option<Value>` — a JSON schema the host must render as a form, plus `ElicitationResult { action, content }` to submit it. An `accept` with the wrong `content` is worse than no card, so this needs a real form renderer, not a bigger `askcard` | a turn that asks a multi-choice question renders buttons |

3.1 is one line and is the reason to do this step at all; 3.2 does not exist and 3.3
is a phase of its own.

### Next phase — resume, and why it is not in this branch

`ResumeSessionOptions` is 32 fields, and every group in it is one `connect()`
already builds — so the config half is reuse, not new work. What it still needs is
not config: a `POST /resume {convo}` route that maps our conversation id to the
runtime's session id, the `Arc<Live>` swap the restart path already does, and a
page surface that uses it when a conversation is opened. Half of that is UI, so it
gets its own pass rather than being bolted onto the end of the migration.

### Step 4 — cheap plumbing, one pass

`policy.allowed_models` (refuse an unservable model before the turn fails),
`filesystem.additional_directories` (a second readable root without widening the
fence), `mcp.disable` (stop the enable-then-disable dance), and
`inference.enable_citations`. All S; batch them, they touch `connect()` only.

### Step 5 — merge

Steps 1–2 are one PR (`feat/copilot-sdk-v2`); 3 and 4 follow separately so a
capability problem cannot hold the migration hostage. PR #107 is untouched either
way.

### Not planned, on purpose

`agents.*` (L, and config discovery is off by design), `state.memory_store` (L,
product decision), resume via `ResumeSessionConfig` (L, the callback set has to
be re-supplied or the resumed session silently differs), and everything in
"Refuse" in `docs/CHAT_SDK_GAPS.md`.

## Second pass — three call sites fixed, and the builders are not what we assumed

**The generated module becomes readable after a build.** `rpc.rs` and
`session_events.rs` are facades, but `cargo check` writes the real thing beside
them: `src/generated/` appears with 18 files (3.6 MB), `api_domain_session.rs`
799 KB, `rpc.rs` 919 KB, `session_events.rs` 657 KB. So every question of the
form "what is this type called now" has a cheap answer — compile once, then read
— and the compiler is no longer the only oracle.

Fixed, from that source:

- `/settings` patch regrouped. `organization_custom_instructions` →
  `instructions.organization_text`, `enable_session_store` →
  `state.enable_cross_session_store`, `reasoning_effort` →
  `inference.reasoning_effort`. The three group structs derive `Default`, so the
  patch stays a struct-update expression.
- `McpEnableRequest` / `McpDisableRequest`: `expected_installation_id:
  Option<String>` — `None` here, because we attach the server at session creation
  and hold no receipt to match.
- `UserInputHandler::handle` gained `request_id: Option<RequestId>`
  (`types::RequestId`). Answered through our own bus, so it is ignored — but it is
  also how the page *would* answer through `session.ui` if the bus ever goes.

**The builder surface is the real news, and it is worse than the field diff.**
`CreateSessionOptions` offers 31 builders and every one is group-level:
`with_system_message_transform`, `with_session_id`, `with_name`, `with_model`,
`with_client_name`, `with_system_message`, `with_diagnostics`,
`with_working_directory`, `with_cloud`, `with_inference`, `with_policy`,
`with_events`, `with_interaction`, `with_tools`, `with_canvases`,
`with_extensions`, `with_agents`, `with_skills`, `with_mcp`,
`with_config_directory`, `with_enable_config_discovery`, `with_filesystem`,
`with_state`, `with_instructions`, `with_hooks`, `with_git`, `with_github`,
`with_plugins`, `with_permissions`, `with_execution`, `with_experiments`.

These are **gone**, and `connect()` calls every one of them: `with_provider`,
`with_mcp_servers`, `with_system_message`, `with_infinite_sessions`,
`with_permission_handler`, `approve_all_permissions`, `approve_permissions_if`,
`with_user_input_handler`, `with_hooks`, `with_skill_directories`. Plus
`ClientOptions::with_base_directory` / `with_program`, which no longer exist on a
client built by a constructor.

So `connect()` is not a rename pass. The 160 lines that build a `SessionConfig`
by chaining become something that constructs group structs — `SessionMcpOptions
{ servers }`, `SessionToolOptions { available_tools, excluded_tools, search,
large_output }`, `SessionEventOptions { streaming,
include_sub_agent_streaming_events }`, `SessionStateOptions { infinite_sessions,
memory, enable_cross_session_store }`, `SessionSkillOptions { enabled,
directories }`, `SessionInstructionOptions { organization_text }`,
`SessionFilesystemOptions { track_file_changes }`, `SessionPermissionOptions`
with the "approve everything" mode expressed as data. Two decisions in that block
are behaviour, not plumbing — the permission mode (the path fence's twin) and the
`Fence` hook — so they get read before they are retyped, not after.

The upside of the whole pass: the tree is no longer 8 unknowns deep. It is one
function, and both remaining errors are in it.

## Status

Branch `feat/copilot-sdk-v2`, cut from the green `feat/chat-html-vocabulary`
tree so nothing here can break PR #107.

Done: dependency renamed to `copilot-sdk = "2.0.1"` with the same feature
posture, `github_copilot_sdk` → `copilot_sdk` at all 22 sites, first compile run
and its 8 errors catalogued above, then all five call-site fixes from the
generated sources, then `connect()` itself.

**It compiles: `cargo check` exit 0, no warnings, and the 58 CLI tests pass.**
That is the migration's gate met, not its verification — no live turn has run on
2.0.1 yet, which is step 2 of the plan below.

What the rewrite actually turned on, in `connect()`: the ten groups replaced the
flat config one-for-one, and every group is `#[non_exhaustive]`, so the group
builders are the only door (no struct literals, and the parent's
`with_<group>` setters take a closure: `.with_mcp(|m| m.with_servers(servers))`).
An earlier draft of this note said `approve_permissions_if` was gone — it is
not: `SessionPermissionOptions` kept it, which is why no predicate adapter was
needed.

One capability came with the migration: `inference.allow_multiple_tool_calls`,
now a Runtime-tab switch (`parallel_tools`, default on, creation-time so a flip
reports `restart: true`).

The measured totals that re-baseline the gaps inventory are in
`docs/CHAT_SDK_GAPS.md` (fifth pass).


## Running it — what the compiler could not tell us

Compiling was half of it. Three defects only appear at startup, and two of them
are the kind that fail *quietly* in a later code review:

**1. `RuntimeConnection::for_default` picks in-process.** It follows the
deployment's own build, which here is `Transport::InProcess`, and this build has
no `in-process` feature:

```
Error: could not start the Copilot runtime: Transport::InProcess requires the `in-process` Cargo feature
```

The silent half: `child_process_options_mut()` returns `None` for a runtime that
is not spawned, so the program path assigned through it is simply never set — the
code compiles, reads correctly, and does nothing. Fixed by building the transport
explicitly:

```rust
let mut spawn = copilot_sdk::SpawnedProcessOptions::new();
spawn.owned = owned;                     // base directory + idle timeout
spawn.program = CliProgram::Path(bin);   // webrain install copilot
options.with_runtime(RuntimeConnection::SpawnedStdio(spawn));
```

**2. The runtime itself had to move with the SDK.** 2.0.1 pins Copilot CLI
**1.0.114** (`cli-version.txt` in the crate carries the tag *and* the per-platform
digests); we were pinned at **1.0.105**:

```
Error: could not start the Copilot runtime: version mismatch: server=3, supported=4–4
```

`install.rs` now pins 1.0.114 with those digests, and `find_named` sorts
candidate directories newest-first — it returned the *first* match in filesystem
order, so a stale version directory winning was only a matter of listing order,
which is how a bumped pin looks applied and stays old.

**3. BYOK needs CAPI switched off.** `inference.default_provider` is refused
outright otherwise:

```
session construction failed: inference.defaultProvider requires github.capi.enabled to be false
```

`.with_github(|g| g.with_capi(CapiSessionOptions::new().with_enabled(false)))`.

### What is now verified by actually running it

| claim | evidence |
|---|---|
| the session creates on 2.0.1 | `webrain chat ┬╖ serving on http://127.0.0.1:8787/` |
| routes survive the regroup | `GET /settings` (with `parallel_tools: true`), `/name`, `/mode`, `/jobs` all answered |
| a turn reaches the provider | `POST /turn` → `{"started":true}`, then `model.model_call_failure` |
| the token caps survived the move | OpenRouter's 402 quotes **"requested up to 4096 tokens"** — the runtime default is 64000, so `ProviderModelConfig.max_output_tokens` is in force |
| errors still surface | the provider's message arrived as `model.model_call_failure` plus a `hook.start` of `errorOccurred` |

Still unproven: **the permission path**. The turn died at the provider (402, the
key is out of credits) before any tool call, so no approval card was raised. That
is the one thing left to check on a funded key, and it is the one that fails
silently if `permissions.on_request` were dropped.

## Evidence notes

- `github_repo` (semantic) answers `404` for `github/copilot-sdk`, and
  `github_text_search` / `search_code` return empty for every query tried
  (`SessionUpdateOptionsParams`, `session.options.update`,
  `ToolsExecuteRequest`, `CreateSessionOptions`, `SessionPermissionOptions
  mode`), re-checked 2026-11-21. The repo is not readable through those tools, so
  no fact above came from them.
- What does answer: the extracted crate plus one `cargo check`, after which
  `src/generated/` holds the protocol (see "second pass"). That is the only
  source used for type and field names.
