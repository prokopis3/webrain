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
- `SessionUpdateOptionsParams`' grouped fields (the `/settings` patch path).
- What `expected_installation_id` must carry for `session.mcp.enable/disable`.
- Whether the generated rpc request types keep their 1.0.28 names; they are
  produced at build time (private `generated` module), so only the compiler can
  answer.

## Status

Branch `feat/copilot-sdk-v2`, cut from the green `feat/chat-html-vocabulary`
tree so nothing here can break PR #107.

Done: dependency renamed to `copilot-sdk = "2.0.1"` with the same feature
posture, `github_copilot_sdk` → `copilot_sdk` at all 22 sites, first compile run
and its 8 errors catalogued above.

Not done: the `connect()` rewrite, the five call-site fixes, and any runtime
verification. **This branch does not compile yet** — it is a measured start, not
a migration.
