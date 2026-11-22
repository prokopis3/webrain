# Chat capabilities — what is still open

Measured 2026-11-21 against `copilot-sdk` 2.0.1 on `feat/copilot-sdk-v2`. This was
the full audit; everything it listed as shipped is in `CHANGELOG.md` and the git
history, so what is left here is only what is still missing — plus the one thing
that is built and has never been run.

## Open

| # | what | today | first step | size |
|---|---|---|---|---|
| 1 | `session.quota_observation`, `session.session_limits_changed` | **CLOSED 2026-11-22** — all three now say something. `quota_observation` prints the provider's own balance from `observation` (`availableQuantity` of `entitledQuantity`, with `-1` read as unlimited rather than as a number), plus `capacityState` and `accessState` when they are not the happy value — because a zero quantity does **not** mean exhausted, the verdict is printed beside the number instead of being inferred from it. `session_limits_changed` prints `sessionLimits.maxAiCredits`, and `session_limits_exhausted.requested` says how much of it was used | — |
| 2 | the worker's own turns | a delegated agent leaves a chip while it runs and one closing cost line; the transcript shows the parent's `task` row, not what the worker said | a nested transcript per `toolCallId` — the events carry the id, the page has no second stream to paint | L |
| 3 | agent picker | `read_agent`/`write_agent`/`list_agents` are served, so the model can manage definitions, but a person cannot | the `agents` group on the config (`default`, `selected`, `included_builtin_agents`) plus an editing surface | L |
| 4 | `/command` results that want a surface | **CLOSED 2026-11-22** — all eight variants are handled: `Text`/`Completed` → note, `AgentPrompt` → composer, `SetModel` → the existing `/switch` (which knows whether the context survives), `AddTimelineEntry` → note plus its `prefill`, `ShowDialog` → opens the picker the page already has, `SelectSubcommand` → its options listed with the parent left in the field, `SetPlanModel` → note. Measured on a BYOK session: `/model` answers `Text`, because a model that is not registered cannot be routed — the `SetModel` variant is a Copilot-plan path, not this one | — |
| 5 | MCP sampling, *answered* | handled as the protocol's reject — the hang is gone, the completion is not run | a non-empty `UIHandlePendingSamplingResponse` upstream, or the raw-call route to `session.mcp.executeSampling` with the event's params forwarded whole | M, and it spends credits |
| 6 | citations | `inference.enable_citations` is off and nothing would render a citation | turn it on *and* build the renderer, in that order | M |
| 8 | approval rules that survive the turn | our handler answers approve/deny per request, so there is no "always allow this directory" | **blocked, measured 2026-11-22**: `permissions.get_capabilities()` answers `{assistedApproval, bypassPermissions}` and nothing else — no rule vocabulary. `modify_rules` takes `{kind, argument}` where `kind` is documented only by example (`"such as Shell or GitHubMCP"`), and a rule written with a guessed kind either matches nothing or matches too much | blocked until the vocabulary is published |
| 9 | `mcpprompts` / `mcpresources` | **SHIPPED 2026-11-22** — `GET /mcp/items` asks every connected server through `mcp.prompts().list()` / `mcp.resources().list()` and the MCP tab renders whatever comes back, read-only (a prompt is invoked by the model, not by a switch). Measured: our own server answers `MCP error -32601: Unknown method`, which is an *absence* rather than a failure, so it is not reported — the route returns `{prompts: [], resources: [], errors: []}` | — |
| 10 | `dynamic_context` | read as "per-turn context" in the old audit — it is not: `set_config` takes `{branch, repository, store_path}`, a **context board** for a repository | nothing to build for a chat; drop the row if it ever resurfaces | — |

The remaining ~42 uncalled session namespaces are not a backlog: they are
`managedsettings`, `plugins*`, `sandbox`, `remote`, `canvas*`, `workflow*` and
`accounts*`, which serve a Copilot-plan or enterprise host. A BYOK local chat has
no policy to receive, no marketplace to install from, and already has a boundary
of its own (`chat_hooks::Fence`).

Two deliberate non-rows, so nobody "fixes" them:

- `permission.requested` stays unhandled — it is the event twin of the request our
  own `WebPermission` card already draws, and handling it would show every
  approval twice.
- `/command` uses `session.commands.invoke`, not `execute`. `execute` answers
  `No client found for command` for all 29 built-ins; it is the path for a host
  that registered handlers.
- "Model picker" is **not** an open row, corrected 2026-11-22: the page carries a
  live per-provider catalogue (`/models`, `loadModels`) and `/switch` keeps the
  context when the endpoint is the same. Switching *provider* still rebuilds the
  session, which is not a gap — another endpoint is another session.

## What is verified, and what is not

Verified by running it: the session creates on 2.0.1; `/settings`, `/name`,
`/mode`, `/jobs`, `/runtime/sessions`, `/catalog` (with the runtime's 29 commands),
`/command` (`/cwd`, `/usage`, `/env` returned text), `/queue` and `/resume` all
answer; and the provider's 402 quoting "requested up to 4096 tokens" proves the
migrated model caps are in force.

Not verified, in order of how much it matters:

1. **The permission path.** Every turn so far has died at the provider (the key is
   out of credits) before a tool call, so no approval card has been raised on
   2.0.1. Until a funded key raises one, treat `permissions.on_request` as
   compiled-but-unproven — it is the failure that looks like success.
2. **Elicitation and sampling.** Both handlers compile and have unit-tested
   coercion, but no MCP server here has raised either event, so neither has run
   against a real payload.
3. **Stop-and-send.** `/stop` now clears the busy flag, and the composer's new
   button depends on it; the abort → send sequence has never been exercised.
4. **A sub-agent run.** All six `subagent.*` arms are pinned to the wire ids by
   `check-subagent-events.mjs`, but nothing has delegated anything on 2.0.1.
