# Chat capabilities — what is still open

Measured against `copilot-sdk` 2.0.1 on `feat/copilot-sdk-v2`, last pass
2026-11-23. Everything that shipped is in `CHANGELOG.md` and the git history; this
file holds only what is still missing, and it is short because most of it turned
out to be blocked upstream rather than unbuilt.

**What is open, and nothing else.** Seven rows were on this list; five shipped and are
described in `CHANGELOG.md` and the git history, so they are not repeated here. Two
remain, and both are blocked by the SDK rather than by this repo — the evidence for
that is the symbol table below, not a hunch.

## Open

| # | what | why it is still open |
|---|---|---|
| 1 | the worker's own transcript | `assistant.message_delta` carries only `{deltaContent, messageId}` — no agent or parent id — so a streamed line cannot be attributed *while it streams*. It is not blocked, it is deferred: `delta.messageId` joins to `assistant.message.turnId`, and `assistant.turn_end` carries `parentTaskToolCallId`, so a child's text can be re-filed under its `task` row when that turn **ends**. Doing that means buffering per-message text on the hottest path in the page, so it stays open until that is worth its risk |
| 4 | MCP sampling, *answered* | handled as the protocol's reject, so the hang is gone and the completion is not run. The typed path cannot carry it: `McpExecuteSamplingRequest` is an **empty struct** ("treated as opaque at the schema layer") while the params that matter — the MCP `createMessage` messages — are what it would have to hold, so `executeSampling` would ask the runtime to run an inference with no prompt. Answering needs either a non-empty struct upstream or a hand-built raw call, which spends credits per request |

## What the SDK actually offered

Every claim in this file was checked against the crate source, not its prose. Where
the truth lives matters, because it is not where the names suggest: in
`copilot-sdk-2.0.1`, `src/<name>.rs` are **7-line `include!` shims** and the real
definitions are in `src/generated/`. Grepping the shim reports line numbers that do
not exist in the file it printed.

| Symbol | What it settled |
|---|---|
| `McpStartServerRequest { config: Option<Value>, expected_installation_id, server_name }` + `session.mcp.start_server` | `config: None` starts a server **with the already-registered configuration**, in the live session. This is what closed #3 — `mcp.config.reload` only re-reads the file, and the "marked internal" note that row was blocked on belongs to a different call |
| `McpServerStatus` (`connected` / `failed` / `needs-auth` / `pending` / `disabled` / `stopped` / `not_configured`) | Why `attached` must compare `status == "connected"`: a spawn failure still appears in `list()`, so a presence check reports success |
| `session.agent.select` / `deselect` / `agent.get_current` | Selection moves on a **live** session, so #2 needed no restart path — and `get_current` is what the reply must read back, not the request's name |
| `SessionInferenceOptions::with_enable_citations` + `AssistantMessageData.citations: Option<Citations>` | The typed payload exists (`Citations { sources[], spans[] }`), which is what made #5 possible instead of "markers nothing can parse" |
| `McpExecuteSamplingRequest {}` (empty) + `McpExecuteSamplingResult = HashMap<String, Value>` | Why #4 is still open: the *result* is free-form but the *request* is empty, so the MCP `createMessage` messages have nowhere to go. The runtime's own conversion needs params the typed struct cannot carry |
| `AssistantMessageDeltaData { delta_content, message_id }`, `AssistantTurnEndData.parent_task_tool_call_id` | The exact join behind #1's deferral: `delta.messageId` → `assistant.message.turnId` → `turn_end.parentTaskToolCallId` |
| `permissions.get_capabilities()`, `modify_rules { kind, argument }` | No rule vocabulary, and `kind` is documented only by example — which is why #6 stores what the card carries instead |

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

The per-route measurements for everything that shipped live in the commits that
shipped it (each `CHANGELOG.md` entry carries the reply it was measured by), so
they are not repeated here. What is left is what has **never** been seen working,
which is the part that matters when deciding whether to trust this page:

Not verified, in order of how much it matters:

1. **The permission path.** Every turn so far has died at the provider (the key is
   out of credits) before a tool call, so no approval card has been raised on
   2.0.1. Until a funded key raises one, treat `permissions.on_request` as
   compiled-but-unproven — it is the failure that looks like success. This is also
   why the standing-approval store is proven by unit tests (key shape, file round
   trip, managed-never-auto-answered) and by its routes, but not yet by a card
   that was actually skipped.
2. **Citations on a real turn.** The switch, the restart report, the persisted
   value and the page's strip are in place; no funded turn has run with it on, so
   `Citations` has not been seen populated by a provider. The BYOK providers here
   may simply never emit it — the strip draws nothing in that case rather than
   claiming sources.
3. **Elicitation and sampling.** Both handlers compile and have unit-tested
   coercion, but no MCP server here has raised either event.
4. **Stop-and-send.** `/stop` clears the busy flag and the composer's button
   depends on it; the abort → send sequence has never been exercised.
5. **A sub-agent run.** All six `subagent.*` arms are pinned to the wire ids by
   `check-subagent-events.mjs`, but nothing has delegated anything on 2.0.1.
6. **The turn totals.** `webrain.usage` now sends the difference from the snapshot
   taken at the previous turn's end, and the panel sums per-call usage; both are
   one turn's worth, but neither has been compared against a funded turn's bill.
