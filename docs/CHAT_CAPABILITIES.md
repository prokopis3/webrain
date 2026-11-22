# Chat capabilities — what is still open

Measured against `copilot-sdk` 2.0.1 on `feat/copilot-sdk-v2`, last pass
2026-11-22. Everything that shipped is in `CHANGELOG.md` and the git history; this
file holds only what is still missing, and it is short because most of it turned
out to be blocked upstream rather than unbuilt.

## Open

| # | what | why it is still open |
|---|---|---|
| 1 | the worker's own turns | `assistant.message_delta` carries only `{deltaContent, messageId}` — no agent or parent id — so a streamed line cannot be attributed to a sub-agent. `assistant.turn_end` does carry `parentToolCallId`, so totals can be attached to the `task` row, but a nested transcript needs the deltas to say whose they are |
| 2 | agent picker | the list and the create form are shipped (`GET /agents` reads `agents.discover()`, `POST /agents/new` writes `.github/agents/<slug>.md`); *selecting* one is `agents.selected` on the session config, which is creation-time. A picker needs a restart path, and there is none |
| 3 | MCP server add | `session.mcp.reloadWithConfig` exists but its config field is documented as **"marked internal: an in-process runtime shape (reloadMcpServers throws over the wire)"** — an out-of-process client cannot add a server at runtime. Start/stop of what the session already attached is what is possible |
| 4 | MCP sampling, *answered* | handled as the protocol's reject, so the hang is gone and the completion is not run. Answering needs a non-empty `UIHandlePendingSamplingResponse` upstream — it is an empty struct — or a raw `Client::call` into `session.mcp.executeSampling`, which spends credits per request |
| 5 | citations | `inference.enableCitations` exists; there is no typed citation payload for a message. The only `citations` field in the protocol is a `String` on the memory-store request, so enabling it asks every model for markers nothing can parse |
| 6 | approval rules that survive the turn | `permissions.get_capabilities()` answers `{assistedApproval, bypassPermissions}` and no rule vocabulary. `modify_rules` takes `{kind, argument}` with `kind` documented only by example ("such as Shell or GitHubMCP"), and a rule written with a guessed kind either matches nothing or matches too much |

## Not rows, deliberately

- `permission.requested` stays unhandled — it is the event twin of the request our
  own `WebPermission` card already draws, and handling it would show every
  approval twice.
- `/command` uses `session.commands.invoke`, not `execute`. `execute` answers
  `No client found for command` for all 29 built-ins; it is the path for a host
  that registered handlers.
- The ~42 uncalled session namespaces are not a backlog: `managedsettings`,
  `plugins*`, `sandbox`, `remote`, `canvas*`, `workflow*`, `accounts*` serve a
  Copilot-plan or enterprise host. A BYOK local chat has no policy to receive, no
  marketplace to install from, and already has a boundary of its own
  (`chat_hooks::Fence`). `dynamic_context` is a repository context board
  (`branch`, `repository`, `store_path`), not per-turn context injection.

## What is verified, and what is not

Verified by running it: the session creates on 2.0.1; `/settings`, `/name`,
`/mode`, `/jobs`, `/runtime/sessions`, `/catalog` (with the runtime's 29 commands),
`/command` (`/cwd`, `/usage`, `/env` returned text), `/queue`, `/resume`,
`/mcp/items`, `/permission-rules` and `GET /agents` all answer; `POST /agents/new`
wrote a definition that the runtime's own `discover()` then listed; `POST
/skills/new` wrote a skill that `/catalog` showed immediately and that a
`../escape` name was refused for; and the provider's 402 quoting "requested up to
4096 tokens" proves the migrated model caps are in force.

Not verified, in order of how much it matters:

1. **The permission path.** Every turn so far has died at the provider (the key is
   out of credits) before a tool call, so no approval card has been raised on
   2.0.1. Until a funded key raises one, treat `permissions.on_request` as
   compiled-but-unproven — it is the failure that looks like success.
2. **Elicitation and sampling.** Both handlers compile and have unit-tested
   coercion, but no MCP server here has raised either event.
3. **Stop-and-send.** `/stop` clears the busy flag and the composer's button
   depends on it; the abort → send sequence has never been exercised.
4. **A sub-agent run.** All six `subagent.*` arms are pinned to the wire ids by
   `check-subagent-events.mjs`, but nothing has delegated anything on 2.0.1.
5. **The turn totals.** `webrain.usage` now sends the difference from the snapshot
   taken at the previous turn's end, and the panel sums per-call usage; both are
   one turn's worth, but neither has been compared against a funded turn's bill.
