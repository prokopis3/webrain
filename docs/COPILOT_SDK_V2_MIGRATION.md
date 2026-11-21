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

## Still to read before writing `connect()`

- `NamedProviderConfig` full field list, and whether the old `ProviderConfig`
  nests inside it or was replaced.
- Whether the generated rpc request types keep their 1.0.28 names.

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
generated sources. **8 errors → 2**, and both survivors are the same line of
`connect()`: the `SessionConfig` import and `ClientOptions::default()`.

Not done: the `connect()` rewrite — blocked on measuring the group structs, not
on anything external — and any runtime verification. **This branch does not
compile yet.** The measured totals that re-baseline the gaps inventory are in
`docs/CHAT_SDK_GAPS.md` (fifth pass).
