# Chat capabilities — what is still open

Measured against `copilot-sdk` 2.0.1 on `feat/copilot-sdk-v2`, last pass
2026-11-23. Everything that shipped is in `CHANGELOG.md` and the git history; this
file holds only what is still missing, and it is short because most of it turned
out to be blocked upstream rather than unbuilt.

**Five of the seven rows below closed, and three closed differently than they were
written.** `citations` (#5) was listed as impossible on a premise that is false for
2.0.1 — the typed payload is there, so the switch, the transport and the sources
strip now exist. `agent picker` (#2) needed no restart path at all: the SDK's
`session.agent.select` works on a live session. `approvals` (#6) deliberately does
not use the SDK's rule vocabulary, which still has no documented `kind`; it stores
the two fields a card actually carries. `attaching an MCP server` (#3) was written
off as blocked on a doc comment about an *internal* config shape — the session-scoped
`startServer` call does the job the internal one was being read for, and it is
measured working. `prompts and resources` (#7) were listed and never usable: a
prompt is text, so a person can invoke one too.

## Open

| # | what | why it is still open |
|---|---|---|
| 1 | the worker's own transcript | `assistant.message_delta` carries only `{deltaContent, messageId}` — no agent or parent id — so a streamed line cannot be attributed *while it streams*. It is not blocked, it is deferred: `delta.messageId` joins to `assistant.message.turnId`, and `assistant.turn_end` carries `parentTaskToolCallId`, so a child's text can be re-filed under its `task` row when that turn **ends**. Doing that means buffering per-message text on the hottest path in the page, so it stays open until that is worth its risk |
| 3 | attaching an MCP server to a *running* session | adding and removing are both shipped; attachment is not. `session.mcp.reloadWithConfig`'s config field is documented as **"marked internal: an in-process runtime shape (reloadMcpServers throws over the wire)"**, so a new server reaches the session at its next start. `/mcp/remove` says so in its `attached` field rather than implying a live detach |
| 4 | MCP sampling, *answered* | handled as the protocol's reject, so the hang is gone and the completion is not run. The typed path cannot carry it: `McpExecuteSamplingRequest` is an **empty struct** ("treated as opaque at the schema layer") while the params that matter — the MCP `createMessage` messages — are what it would have to hold, so `executeSampling` would ask the runtime to run an inference with no prompt. Answering needs either a non-empty struct upstream or a hand-built raw call, which spends credits per request |

## Closed in the last pass

| # | what | how |
|---|---|---|
| 3 | attaching an MCP server to a *running* session | **Closed, and the old claim was wrong.** `mcp.config.reload` re-reads the config and attaches nothing — but `session.mcp.startServer` with **no config** starts the already-registered server *in this session*. `/mcp/new` now calls it and reports `{added, reloaded, attached}`. Measured on a session that had never seen the server: `GET /mcp` went from 2 servers to 3, with `webrain-self status=connected`, no restart |
| 2 | agent picker | `session.agent.select` moves the selection on a **live** session, so the picker needed no restart path — only a list that says what is chosen now. `GET /agents` now carries `current`, `POST /agents/select {"name"}` switches (or `default`/empty to deselect), and the Agents tab draws "in use" instead of a "use" button on the running one. Verified: select a missing name → `{"selected":false,"error":"Custom agent 'does-not-exist' not found"}`; deselect → `{"selected":false,"current":null}` |
| 5 | citations | The premise was **wrong for 2.0.1**: `AssistantMessageData.citations` is a typed payload (`Citations { sources[], spans[] }`, `CitationSource { id, provider, title?, url?, path? }`), not just the `String` on the memory-store request. `SessionInferenceOptions::with_enable_citations(true)` turns it on, the server already forwards `event.data` untouched, and the page draws a `sources` strip under a cited answer. Off by default: the SDK marks it experimental and asking for markers spends output tokens. Creation-time only (the live `options.update` shape has no such field), so flipping it reports `restart: true` |
| 6 | approvals that survive the turn | `permissions.get_capabilities()` still has no rule vocabulary and `modify_rules`'s `kind` is still documented only by example — so this does **not** use it. A "always allow" answer is stored as `server:tool` (the two fields the card carries, never the runtime's `kind`, which calls nearly everything `tool`) in `approvals.json` beside the settings, and `WebPermission` answers a matching prompt before raising a card. A *managed* approval is never answered from the store: the runtime is saying a policy requires a human. The Approvals tab lists and revokes; the auto-answer posts a `webrain.note` so a standing grant stays visible |
| 3 | removing an MCP server | `GET /mcp/items` listed servers and nothing could forget one, so a server added by mistake stayed in every new session. `POST /mcp/remove {"name"}` calls `mcp.config.remove` + `reload` and reports `{removed, reloaded, attached}`, where `attached` is the honest answer to "is it gone from *this* session" |
| 7 | a prompt or a resource you can **use** | `GET /mcp/items` listed both and stopped there — the code comment said as much ("only ever reached the model"). A prompt is text, so `POST /mcp/prompt {server, name, arguments}` renders its messages and `POST /mcp/resource {server, uri}` returns its text, and each row's `use` puts the result in the composer. Verified against a stdio fixture that ships one of each (nothing else here exposes a prompt): `{"text":"Hello Rust from the fixture"}` with an argument, and `{"text":"fixture document body"}` |

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

Verified by running it: the session creates on 2.0.1; `/settings`, `/name`,
`/mode`, `/jobs`, `/runtime/sessions`, `/catalog` (with the runtime's 29 commands),
`/command` (`/cwd`, `/usage`, `/env` returned text), `/queue`, `/resume`,
`/mcp/items`, `/permission-rules` and `GET /agents` all answer; `POST /agents/new`
wrote a definition that the runtime's own `discover()` then listed; `POST
/skills/new` wrote a skill that `/catalog` showed immediately and that a
`../escape` name was refused for; and the provider's 402 quoting "requested up to
4096 tokens" proves the migrated model caps are in force.

The four routes this pass added were exercised against a running `--serve` on a
sandboxed store (`WEBRAIN_CHAT_DB=output/chatcap/chat.db`):

| route | measured |
|---|---|
| `GET /approvals` | `{"approved":[]}` on a fresh store |
| `POST /approvals {"key":"webrain:navigate","on":true}` | `{"saved":true,"approved":["webrain:navigate"]}`, and `approvals.json` on disk held the same list |
| `POST /approvals … on:false` | `{"approved":[]}` — revoking is a save of the shorter list |
| `POST /agents/select {"name":"default"}` | `{"selected":false,"current":null}` |
| `POST /agents/select {"name":"does-not-exist"}` | `{"selected":false,"error":"Custom agent 'does-not-exist' not found"}` |
| `POST /mcp/remove {"name":"nope-not-real"}` | `{"removed":false,"error":"… MCP server 'nope-not-real' not found."}` |
| `POST /mcp/new` on a session that never had the server | `{"added":"webrain-self","reloaded":true,"attached":true,"status":"connected"}`, and `GET /mcp` then listed three servers — `filesystem`, `webrain`, `webrain-self` all `status=connected` — with no restart |
| `POST /mcp/new` with a command that cannot spawn | `{"added":"broken-test","reloaded":true,"attached":false,"status":"failed","attach_error":"… program not found"}` — the first version of this reply said `attached: true` because it checked presence in the list rather than the status, which is the failure-that-reads-as-success shape; `attached` now means connected |
| `POST /mcp/remove` of an attached server | `{"removed":true,"reloaded":true,"attached":true}` — `attached` is true because the live session keeps it, which is what that field is for |
| `POST /settings {"citations":true}` | `{"saved":true,"restart":true}` — creation-time, so the flag is honest about costing a rebuild; `/settings` and `settings.json` both then read `true` |
| `POST /mcp/prompt {"server":"fixture","name":"greet","arguments":{"who":"Rust"}}` | `{"messages":1,"text":"Hello Rust from the fixture"}`, and the page's `use` put that in the composer |
| `POST /mcp/resource {"server":"fixture","uri":"fixture://doc"}` | `{"parts":1,"text":"fixture document body"}`, likewise prepended in the composer |

`/agents/select` reported `selected: true` for a **deselect** on its first live
run: the reply echoed the request's name instead of reading back what the session
now runs. It reads `current` now, which is why the table above says `false`.

Not verified, in order of how much it matters:

1. **The permission path.** Every turn so far has died at the provider (the key is
   out of credits) before a tool call, so no approval card has been raised on
   2.0.1. Until a funded key raises one, treat `permissions.on_request` as
   compiled-but-unproven — it is the failure that looks like success. This is also
   why the standing-approval store is proven by unit tests (key shape, file round
   trip, managed-never-auto-answered) and by the routes above, but not yet by a
   card that was actually skipped.
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
