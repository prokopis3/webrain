# Chat capabilities — what is still open

Measured against `copilot-sdk` 2.0.1 on `feat/copilot-sdk-v2`, last pass
2026-11-23. Everything that shipped is in `CHANGELOG.md` and the git history; this
file holds only what is still missing, and it is short because most of it turned
out to be blocked upstream rather than unbuilt.

**Three of the six rows below closed, and one closed a different way than it was
written.** `citations` (#5) was listed as impossible on a premise that is false for
2.0.1 — the typed payload is there, so the switch, the transport and the sources
strip now exist. `agent picker` (#2) needed no restart path at all: the SDK's
`session.agent.select` works on a live session. `approvals` (#6) deliberately does
not use the SDK's rule vocabulary, which still has no documented `kind`; it stores
the two fields a card actually carries.

## Open

| # | what | why it is still open |
|---|---|---|
| 1 | the worker's own transcript | `assistant.message_delta` carries only `{deltaContent, messageId}` — no agent or parent id — so a streamed line cannot be attributed *while it streams*. It is not blocked, it is deferred: `delta.messageId` joins to `assistant.message.turnId`, and `assistant.turn_end` carries `parentTaskToolCallId`, so a child's text can be re-filed under its `task` row when that turn **ends**. Doing that means buffering per-message text on the hottest path in the page, so it stays open until that is worth its risk |
| 3 | attaching an MCP server to a *running* session | adding and removing are both shipped; attachment is not. `session.mcp.reloadWithConfig`'s config field is documented as **"marked internal: an in-process runtime shape (reloadMcpServers throws over the wire)"**, so a new server reaches the session at its next start. `/mcp/remove` says so in its `attached` field rather than implying a live detach |
| 4 | MCP sampling, *answered* | handled as the protocol's reject, so the hang is gone and the completion is not run. Answering needs a non-empty `UIHandlePendingSamplingResponse` upstream — it is an empty struct — or a raw `Client::call` into `session.mcp.executeSampling`, which spends credits per request |

## Closed in the last pass

| # | what | how |
|---|---|---|
| 2 | agent picker | `session.agent.select` moves the selection on a **live** session, so the picker needed no restart path — only a list that says what is chosen now. `GET /agents` now carries `current`, `POST /agents/select {"name"}` switches (or `default`/empty to deselect), and the Agents tab draws "in use" instead of a "use" button on the running one. Verified: select a missing name → `{"selected":false,"error":"Custom agent 'does-not-exist' not found"}`; deselect → `{"selected":false,"current":null}` |
| 5 | citations | The premise was **wrong for 2.0.1**: `AssistantMessageData.citations` is a typed payload (`Citations { sources[], spans[] }`, `CitationSource { id, provider, title?, url?, path? }`), not just the `String` on the memory-store request. `SessionInferenceOptions::with_enable_citations(true)` turns it on, the server already forwards `event.data` untouched, and the page draws a `sources` strip under a cited answer. Off by default: the SDK marks it experimental and asking for markers spends output tokens. Creation-time only (the live `options.update` shape has no such field), so flipping it reports `restart: true` |
| 6 | approvals that survive the turn | `permissions.get_capabilities()` still has no rule vocabulary and `modify_rules`'s `kind` is still documented only by example — so this does **not** use it. A "always allow" answer is stored as `server:tool` (the two fields the card carries, never the runtime's `kind`, which calls nearly everything `tool`) in `approvals.json` beside the settings, and `WebPermission` answers a matching prompt before raising a card. A *managed* approval is never answered from the store: the runtime is saying a policy requires a human. The Approvals tab lists and revokes; the auto-answer posts a `webrain.note` so a standing grant stays visible |
| 3 | removing an MCP server | `GET /mcp/items` listed servers and nothing could forget one, so a server added by mistake stayed in every new session. `POST /mcp/remove {"name"}` calls `mcp.config.remove` + `reload` and reports `{removed, reloaded, attached}`, where `attached` is the honest answer to "is it gone from *this* session" |

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
| `POST /settings {"citations":true}` | `{"saved":true,"restart":true}` — creation-time, so the flag is honest about costing a rebuild; `/settings` and `settings.json` both then read `true` |

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
