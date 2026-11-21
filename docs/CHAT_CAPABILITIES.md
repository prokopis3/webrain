# Chat capabilities — what is still open

Measured 2026-11-21 against `copilot-sdk` 2.0.1 on `feat/copilot-sdk-v2`. This was
the full audit; everything it listed as shipped is in `CHANGELOG.md` and the git
history, so what is left here is only what is still missing — plus the one thing
that is built and has never been run.

## Open

| # | what | today | first step | size |
|---|---|---|---|---|
| 1 | `session.quota_observation`, `session.session_limits_changed`, `session_limits_exhausted.*` | nothing on the page; `usage_checkpoint` already shows spend as it happens | a renderer for `ProviderQuotaState`, which is provider-shaped and has no shared vocabulary yet | S each |
| 2 | the worker's own turns | a delegated agent leaves a chip while it runs and one closing cost line; the transcript shows the parent's `task` row, not what the worker said | a nested transcript per `toolCallId` — the events carry the id, the page has no second stream to paint | L |
| 3 | agent picker | `read_agent`/`write_agent`/`list_agents` are served, so the model can manage definitions, but a person cannot | the `agents` group on the config (`default`, `selected`, `included_builtin_agents`) plus an editing surface | L |
| 4 | `/command` results that want a surface | `SetModel`, `SelectSubcommand`, `ShowDialog`, `AddTimelineEntry` come back as "it wants a surface this page does not have yet" | one at a time; `SetModel` is the only one with an existing route (`/settings`) | M each |
| 5 | MCP sampling, *answered* | handled as the protocol's reject — the hang is gone, the completion is not run | a non-empty `UIHandlePendingSamplingResponse` upstream, or the raw-call route to `session.mcp.executeSampling` with the event's params forwarded whole | M, and it spends credits |
| 6 | citations | `inference.enable_citations` is off and nothing would render a citation | turn it on *and* build the renderer, in that order | M |
| 7 | model picker | switching model rebuilds the session | `ClientRpc.models` — the honest source for the allowlist the Runtime tab already has | M |
| 8 | `permissionspaths` / `permissionsurls` / `permissionsfoldertrust` / `permissionsadditionaldirectories` | our handler answers approve/deny per request, so there is no "always allow this directory" that survives the turn | pick one; they are four namespaces for one idea | M |
| 9 | `mcpprompts` / `mcpresources`, `dynamiccontext` | an MCP server's own prompts and resources are not first-class; per-turn context needs a rebuild | lower value than the rows above | M |

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
