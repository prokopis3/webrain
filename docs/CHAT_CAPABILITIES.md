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
| session rpc namespaces | 65 | 22 | 43 |
| client rpc namespaces | 44 | 1 (`sessions`) | 43 |
| event types | 166 | 91 `case` labels | 75 |

"Gap" is arithmetic, not a to-do list: most of the 43 namespaces are enterprise
surface (`managedsettings`, `plugins*`, `sandbox`, `remote`, `workflow*`,
`accounts*`, `canvas*`) that the chat deliberately refuses. The next section is
what is left after that filter.

## Frontend — what a reader cannot see or do today

Measured by diffing the 166 event types against the 91 the page handles, then
dropping the noise families (fusion, canvas, autopilot, schedule, workflow,
managed-settings, `ide.*`, `*_ref` twins). What survives is real:

| # | unhandled events | what the user loses | size |
|---|---|---|---|
| 1 | `exit_plan_mode.requested`, `auto_mode_switch.requested`, `auto_mode_recovery.*` | **Not a hang — checked.** The SDK documents the unset behaviour: `requestExitPlanMode: false` / `requestAutoModeSwitch: false` goes on the wire, so the runtime never asks. What is lost is a *capability*: the agent cannot ask to leave plan mode (our `/mode` route only lets the human switch), and a rate-limit fallback to the auto model never happens | M |
| 2 | `elicitation.requested` / `.completed` | structured questions become nothing. Needs a form renderer, because `ElicitationResult.content` must match `requested_schema` | L |
| 3 | `permission.requested`, `permission.carriedForward` | approvals reach the page only through our own `WebPermission` card, so what the runtime records as carried-forward is invisible — the card shows the request, not the policy that grew from earlier ones | S |
| 4 | `session.tools_updated`, `session.skills_loaded`, `session.mcp_servers_loaded` | the `/mcp` and `/skills` drawers are fetched once, so a mid-session change leaves a stale list — the health *banner* covers status, not the set | S |
| 5 | `command.queued` / `.execute` / `.completed`, `commands.changed` | the runtime's own slash commands never surface; the page's `/` menu is only ours | M |
| 6 | `subagent.selected` / `configured` / `deselected` | `--allow-agents` is a switch with no visible result; no agent picker or per-agent row | L |
| 7 | `sampling.requested` (MCP sampling), `external_tool.requested` | an MCP server asking the model for a completion goes unanswered. We answer external *tools* (`POST /pending`), nothing answers sampling | M |
| 8 | `session.usage_checkpoint`, `session.quota_observation`, `session.session_limits_changed`, `session_limits_exhausted.*` | cost and remaining budget appear only at shutdown, never while spending | S |
| 9 | `system.message`, `session.custom_notification` | runtime messages addressed to the user are dropped | S |
| 10 | `session.compaction_summarized` | the page shows that compaction happened, not what the summary said | S |
| 11 | `tool.user_requested` | a tool asking for user input has no row of its own | S |

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

## Order I would build it

Nothing in this table is a bug — the one row that looked like a hang is a declined
capability, measured. So the order is by cost against what the reader gains:

1. **#3, #4, #8, #9, #10, #11 in one pass** (S each): the page stops lying about
   permission policy, tool lists, spend, runtime messages, compaction and tool
   input. Six small `case` arms, no new plumbing — the events already reach the
   bus.
2. **#1 the interaction handlers** (M): exit plan mode, auto-mode switch and its
   recovery, so the agent can ask instead of being told. Our `/mode` route stays
   the human half.
3. **Resume** (L): reuse `connect()`'s groups on `ResumeSessionOptions`, a
   `POST /resume {convo}` route, and the page calls it when a conversation opens.
4. **#2 elicitation** (L) once a form renderer exists — it is the prerequisite for
   the runtime's structured questions, and `ask_user` already covers the simple
   case.
5. **#5 / #6 / #7** (M/L) as product decisions: runtime commands, agents, MCP
   sampling.

## What is verified, and what is not

Verified by running it: the session creates on 2.0.1, `/settings`, `/name`,
`/mode`, `/jobs`, `/runtime/sessions` all answer, a turn reaches the provider, and
the provider's 402 quoting "requested up to 4096 tokens" proves the migrated
model caps are in force.

Not verified: **the permission path**. Every turn so far has died at the provider
(the key is out of credits) before a tool call, so no approval card has been
raised on 2.0.1. Until a funded key raises one, treat `permissions.on_request` as
compiled-but-unproven — it is the failure that looks like success.
