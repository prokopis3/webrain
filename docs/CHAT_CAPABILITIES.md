# Chat capabilities — what the page and the session do, and what 2.0.1 offers

The current state, measured 2026-11-21 against `copilot-sdk` 2.0.1 on
`feat/copilot-sdk-v2`. This replaces the migration and gap-inventory notes: those
recorded how we got here, which is now in the git history and the CHANGELOG. Every
number below is a count from the sources, not a recollection.

## Where the numbers come from

- SDK surface: the build-generated protocol under the crate's `src/generated/`
  (`cargo check` writes it; `rpc.rs` and `session_events.rs` are facades over it).
- What the page handles: `case "<type>"` labels in `webrain-cli/ui/src/App.svelte`.
- What the backend calls: `.<namespace>()` sites in `webrain-cli/src`.

## The totals

| surface | SDK offers | we use | gap |
|---|---|---|---|
| session rpc namespaces | 65 | 23 | 42 |
| client rpc namespaces | 44 | 1 (`sessions`) | 43 |
| event types | 166 | 91 `case` labels | 75 |

"Gap" is arithmetic, not a to-do list: most of the 43 namespaces are enterprise
surface (`managedsettings`, `plugins*`, `sandbox`, `remote`, `workflow*`,
`accounts*`, `canvas*`) that the chat deliberately refuses. The next section is
what is left after that filter.

## The enterprise families — do they apply to this chat?

Asked directly, so answered directly: **no, and the reason is different for each.**
They are the bulk of the 43 uncalled namespaces, and "uncalled" is not "missing".

| family | why it cannot serve a local BYOK chat |
|---|---|
| `managed_settings` / `managed_settings_permissions` | enterprise policy pushed into a session. The source is Copilot's own service (`github.fetch_managed_settings`), which a BYOK session never talks to — there is no policy to receive |
| `plugins*` / `extensions` / `connectors` / `marketplaces` | an install surface, not a chat capability. Even installed, their tools arrive through MCP, which we already attach |
| `sandbox` (+ `permissions.sandbox`) | the boundary already exists: `chat_hooks::Fence` plus path-fenced tools. A runtime sandbox duplicates it and needs a host we do not ship |
| `remote` / cloud sessions | needs a Copilot subscription by definition; this session is BYOK and `capi` is off |
| `canvas*` | a renderer on the page. `canvases` is reachable in the config, but there is nothing on the panel that can draw one |

So the 43-row gap stays 43 rows, and that number is a feature of the design rather
than a backlog.

## Frontend — what a reader cannot see or do today

Measured by diffing the 166 event types against the 91 the page handles, then
dropping the noise families (fusion, canvas, autopilot, schedule, workflow,
managed-settings, `ide.*`, `*_ref` twins). What survives is real:

| # | unhandled events | what the user loses | size |
|---|---|---|---|
| 1 | `exit_plan_mode.requested`, `auto_mode_switch.requested`, `auto_mode_recovery.*` | **SHIPPED 2026-11-21** — both handlers installed, reusing the existing ask card (`WebInput::ask` was extracted, so plan mode asks with the plan's own actions as buttons and auto mode asks "retry with the auto model?"). Left out: recovery stays off (`auto_mode_recovery` is false, and its default declines) | M |
| 2 | `elicitation.requested` / `.completed` | **SHIPPED 2026-11-21** as a form over the existing `ask_user` card, not a second renderer. `answer_elicitation` walks `requestedSchema.properties` in sorted order, asks one field at a time (`enum` → buttons, `boolean` → yes/no, `integer`/`number` → parsed, `string` → freeform) and coerces each answer with `coerce`, then resolves the request through `ui.handlePendingElicitation`. A field this page cannot draw — object, array, no `type` — declines the whole request instead of submitting a value the server never asked for, which is the failure mode a textarea would have had. The server's own `message` is published first as a `webrain.note`, so the form has context without repeating it per field | L → S shipped, still one field at a time |
| 3 | `permission.requested`, `permission.carriedForward` | **carriedForward SHIPPED 2026-11-21** — a note naming the reused decision, because the card for the new request was the only evidence the policy had grown. `permission.requested` stays unhandled on purpose: it is the event twin of the request our own `WebPermission` card already draws, and handling it would show every approval twice | S |
| 4 | `session.tools_updated`, `session.skills_loaded`, `session.mcp_servers_loaded` | **SHIPPED 2026-11-21** as notes naming what moved and which command re-reads it. Auto-refreshing the drawer would mean exposing the slash-command handler's card builder, which is a refactor, not a fix | S |
| 5 | `command.queued` / `.execute` / `.completed`, `commands.changed` | **SHIPPED 2026-11-21** — correction: `SessionRpcCommands` **does** exist, and so does `session.commands.list`. The page's `/` menu now carries the runtime's own 29 commands (name, description, `experimental ·` marker) after our own, deduped by name from the same list it builds; picking one posts to `/command`, which invokes it and renders whatever came back — text, a prompt placed in the composer, or "it wants a surface this page does not have yet". `commands.execute` was tried first and refused all 29 with `No client found for command`: that path is for a host that registered handlers | S |
| 6 | `subagent.selected` / `configured` / `deselected` | **SHIPPED 2026-11-21** for the visible half. All six `subagent.*` events now have an arm: `started` puts a row in the running-agents chip, `selected` becomes a note naming the definition and its tool count, `configured` writes the child's model and turn shape onto that row while exactly one worker runs, and `deselected` is recognised as empty by contract. The allowlist was also the bug behind "a switch with no visible result": it never served `read_agent` / `write_agent` / `list_agents`, so the hide list had nothing to hide and delegation could spawn without ever seeing what it spawned from. Still missing, deliberately: the worker's own turns. The transcript shows the parent's `task` row and a closing cost line, not a nested conversation | L → S shipped, transcript L |
| 7 | `sampling.requested` (MCP sampling), `external_tool.requested` | **handled 2026-11-21, declined by design.** The hang is gone: the request is resolved through `ui.handlePendingSampling` with no response (the protocol's reject) and the page says which server asked. It cannot be *answered* from this SDK: `UIHandlePendingSamplingResponse` is an empty struct, so a completion we ran could not be returned, and the runtime's own runner takes `McpExecuteSamplingRequest`, also empty — the MCP messages are explicitly opaque at the schema layer. Answering it needs a non-empty response type upstream, or a raw `Client::call` into `session.mcp.executeSampling` with the event's params forwarded whole, which spends credits and cannot be verified here | M → S shipped as a refusal |
| 8 | `session.usage_checkpoint`, `session.quota_observation`, `session.session_limits_changed`, `session_limits_exhausted.*` | **usage_checkpoint SHIPPED 2026-11-21** — spend appears as it happens, in AIU (the runtime reports credits; only the provider knows their price). The other three still wait: `quota_observation` carries a provider-shaped `ProviderQuotaState` with no renderer | S |
| 9 | `system.message`, `session.custom_notification` | **SHIPPED 2026-11-21** — both become notes; the runtime's own words were being dropped | S |
| 10 | `session.compaction_summarized` | **SHIPPED 2026-11-21** — correction: the event carries no summary text, only `success`/`outcome`/`error`, so what was missing was the *failure*, which is now an error row instead of silence | S |
| 11 | `tool.user_requested` | **SHIPPED 2026-11-21** — "waiting on <tool> for your input", where the turn previously just looked stuck | S |

Also design-level, not event-shaped: **no citations** (`inference.enable_citations`
is off and nothing would render them), **no resume** (the strongest capability
gap overall — the sidebar replays a transcript where the runtime could restore a
session), and **no model picker** built on `ClientRpc.models` (switching model
today rebuilds the session).

## Backend — capabilities the session does not reach

Of the 43 uncalled session namespaces, these are the ones that buy something:

| namespace | what it would change | size |
|---|---|---|
| `permissionspaths` / `permissionsurls` / `permissionsfoldertrust` / `permissionsadditionaldirectories` | the fine-grained policy: today our handler answers approve/deny per request, so there is no "always allow this directory" that survives the turn | M |
| `providersmodels` | the model catalog behind a picker — with `inference.models` already populated, the page could switch without a rebuild | M |
| `mcpprompts` / `mcpresources` | an MCP server's prompts and resources as first-class surfaces (a server's own slash commands) | M |
| `dynamiccontext` | inject per-turn context (current CDP target, active profile) without a rebuild — the same job `system_message_transform` does | M |
| `eventlog` / `telemetry` / `debug` | diagnostics the page cannot render anyway until a diagnostic event exists | blocked |

And on the client side, `ClientRpc.models` (list what the provider actually
serves — the honest source for the allowlist we just added) and
`ClientRpc.host` (version and capabilities from the runtime instead of parsed
flags) are the two worth a look.

### What each remaining row needs first (measured 2026-11-21)

So the next pass starts with a step, not a question:

| row | first step | what it costs |
|---|---|---|
| #7 MCP sampling | **declined, see the row above.** What is left is answering it, and that needs a non-empty `UIHandlePendingSamplingResponse` upstream, or the raw-call route to `session.mcp.executeSampling` — money, and nothing funded to prove it | — |
| #5 runtime commands | **done 2026-11-21** — `session.commands.list` for the menu, `session.commands.invoke` for the pick. Verified against the pinned runtime 1.0.114: `/cwd`, `/usage` and `/env` return text, `/model` returns empty text, all four `builtin`. What is *not* wired is acting on the four result kinds that want a surface (model switch, subcommand picker, dialog, timeline entry) | — |
| #6 agents | **done 2026-11-21** — six event arms, `read_agent`/`write_agent`/`list_agents` served only when delegation is on, and `check-subagent-events.mjs` so a seventh id in a future SDK bump fails the build instead of vanishing. What is left is a *picker*, which is the `agents` group on the config plus an editing surface for `write_agent` | L, and only worth it once a funded key can show a worker running |
| #2 elicitation | **done 2026-11-21** — one field at a time over the `ask_user` card, coerced to the declared type, declined when the schema asks for something a card cannot draw. Two unit tests pin the coercion (`yes`→`true`, `"3.5"` is not an integer) and the four field types | — |

## Order I would build it

Nothing in this table is a bug — the one row that looked like a hang is a declined
capability, measured. So the order is by cost against what the reader gains:

1. ~~**#3, #4, #8, #9, #10, #11 in one pass** (S each)~~ — **DONE 2026-11-21**: nine
   `case` arms, no new plumbing. What each left out is named in the row above.
2. ~~**#1 the interaction handlers** (M)~~ — **DONE 2026-11-21**: the agent can ask to
   leave plan mode and to retry on the auto model; `/mode` stays the human half.
3. ~~**Resume** (L)~~ — **SHIPPED 2026-11-21.** `connect()` takes `Option<SessionId>`
   and resumes through `resume_options`, which copies the groups out of the create
   config instead of re-deriving them (two are resume-only twins —
   `SessionFilesystemOptions`, `SessionInstructionOptions` — so each field we set
   is copied by name). `POST /resume` finds the runtime's session for
   `webrain-<convo>` through `sessions.list` and swaps `Arc<Live>` the way
   `reseed` does. **Verified live**: `{"resumed":true,"name":"webrain-…"}`, and
   `/name` still answered afterwards, so the swap leaves a working session behind.
   Not built: opening a *different* conversation's session (the route resumes the
   one the server is on, which is what the sidebar's current thread is).
4. **#2 elicitation** (L) once a form renderer exists — it is the prerequisite for
   the runtime's structured questions, and `ask_user` already covers the simple
   case.
5. ~~**#5 / #6 / #7**~~ — #6 and #5 **DONE 2026-11-21** (agents: six event arms and
the three definition tools; commands: listed and invokable from `/`). **#7 MCP
sampling** stays last and stays unbuilt: its answer path is `mcp.handlePendingSampling`,
and the handler it needs is a second provider call that spends money per request —
unverifiable while the key is out of credits.

## What is verified, and what is not

Verified by running it: the session creates on 2.0.1, `/settings`, `/name`,
`/mode`, `/jobs`, `/runtime/sessions` all answer, a turn reaches the provider, and
the provider's 402 quoting "requested up to 4096 tokens" proves the migrated
model caps are in force.

Not verified: **the permission path**. Every turn so far has died at the provider
(the key is out of credits) before a tool call, so no approval card has been
raised on 2.0.1. Until a funded key raises one, treat `permissions.on_request` as
compiled-but-unproven — it is the failure that looks like success.
