# ADR-004: a GUI for `webrain chat`

- Status: Proposed
- Date: 2026-11-04
- Related: ADR-001 (architecture), ADR-003 (chat over the Copilot SDK)

## Context

`webrain chat` is one prompt, one turn, one exit code. Everything the runtime
knows is already on the wire — `session.subscribe()` receives typed events
(`assistant.message_delta`, `assistant.message`, `tool.execution_start`,
`session.usage_info`, `model.*`, `permission.requested`) — but the CLI flattens
them into three things: stdout for the answer, stderr for `· tool ·` /
`· model ·` lines, and an exit code.

That flattening is correct for a pipe and wrong for a conversation window. A GUI
needs the same events with *meaning*: a tool call is a card that opens and closes,
a permission request is a dialog (not a silent approve), a model call is a cost
meter, and the turn is still running while the user reads.

Three of the SDK's handler slots are currently filled by policy, not by a person:

| SDK slot | Today | What a GUI must do |
|---|---|---|
| `PermissionHandler` | `approve_permissions_if(not_destructive)` | render the request, wait for a click, return approve/deny |
| `UserInputHandler` | not installed | free-form question with optional choices |
| `ElicitationHandler` | not installed | structured form from a JSON schema |

Not installing a handler is a real behaviour, not a gap: the SDK derives the wire
flags from handler presence, and the runtime falls back to its own prompt. In a
GUI there is no other prompt, so the slots must be filled or those turns stall.

## Options

**A. `webrain chat --serve [--port N]`: the binary serves its own UI.**
Raw TCP + HTTP like `webrain_mcp::run_http` (no axum/hyper — this workspace has
none and should not grow one for this), one static HTML/JS page compiled in with
`include_str!` the way `skills.rs` embeds skill files, `POST /turn` to start a
turn, `GET /events` as SSE carrying the raw event stream, `POST /permission` to
answer a pending request.

- Smallest surface that is still a real conversation UI, no build step, no second
  binary, no new dependency.
- Same security posture as the MCP HTTP listener: loopback by default, refuse a
  non-loopback bind without `WEBRAIN_HTTP_TOKEN`.
- The UI is one file a contributor can read.

**B. TUI (`ratatui`) in the existing terminal.** Cheapest to build and test, but
it is not a "real GUI": no clickable cards, no rendered markdown/images, and it
needs a large new dependency for a worse result than A.

**C. OpenAI-compatible `/v1/chat/completions` proxy.** Point Open WebUI or any
chat frontend at webrain. Tempting — it reuses someone else's UI entirely — but
it throws away the tool-call, permission and cost events, which is exactly the
"conversation meaning" this ADR is about. Worth revisiting as an *export*
endpoint later, not as the interface.

**D. Desktop app / VS Code webview.** Correct long term, wrong next step: a
second repository, a second release pipeline, and the same events through a
second transport.

## Decision

Build **A**, in two PRs:

1. **Server + event mapping.** `webrain chat --serve` starts a session, exposes
   `/turn`, `/events` (SSE), `/permission`, `/cancel`, and serves the page. Every
   SDK event maps to exactly one UI element (table below). One turn at a time per
   connection; the runtime allows concurrent `send_and_wait`, the UI does not
   need it yet.
2. **The page.** Static HTML/CSS/JS, no framework, no build. Dark, monospace,
   three panes: transcript, tool timeline, usage meter. Markdown rendered with a
   *whitelist* renderer (no `innerHTML` on model output).

## Event → UI mapping (the part that makes events mean something)

| SDK event | UI element | Rule |
|---|---|---|
| `assistant.message_delta` | streaming text in the assistant bubble | append; the bubble is created on the first delta |
| `assistant.message` | closes the bubble | replaces the streamed text, so a dropped delta self-heals |
| `tool.execution_start` | a card in the timeline: tool name, args collapsed | open → spinner |
| `tool.execution_complete` (or `tool.execution_end`) | same card | closed → verdict icon + duration; failure never hidden |
| `permission.requested` | modal with the exact command/path/url | **blocks the turn**; approve once / deny; the raw `managed_approval_required` marker overrides any remembered choice |
| `session.usage_info` | context meter (current vs limit) | warning colour past 80%, because the next event is compaction |
| `session.usage_checkpoint` | cost delta on the step it belongs to | pairs with the `· model ·` line the CLI already prints |
| `model.call_failure` | inline error card with the provider's own status and message | never a generic "something went wrong" |
| `session.error` | terminal state of the turn | keeps whatever partial answer arrived |
| `session.compaction_start` / `_complete` | a thin divider in the timeline | explains a sudden latency spike instead of looking like a hang |
| `assistant.idle` / `session.idle` | re-enable the composer | the only signal that the turn is over |

Two rules that matter more than the table:

- **Nothing is invented in the UI.** If the runtime did not emit it, the pane is
  empty. A spinner that outlives the turn is worse than no spinner.
- **The permission modal is the security boundary.** Approving from a UI is the
  same authority as `--yes`; it must show the exact action, not a summary.

## What has to change in code

| File | Change |
|---|---|
| `webrain-cli/src/chat.rs` | `--serve` / `--port` flags; refuse `--serve` with `--json` |
| `webrain-cli/src/chat_session.rs` | split `run()` into `run_turn()` (today's body) and a `Session` handle a server can drive; install the three handlers behind a channel instead of the policy closures |
| `webrain-cli/src/chat_server.rs` (new) | listener (borrowing `run_http`'s loopback/token rules), SSE writer, `POST` handlers |
| `webrain-cli/assets/chat.html` (new) | the page, `include_str!`'d |
| `docs/guides/chat.mdx` | "Open it in a browser" section + the security note |
| `docs/reference/cli.mdx` | the two flags |

The handler channels are the only real design work: `PermissionHandler::handle`
is `async` and may outlive the request that triggered it, so it needs a
`tokio::sync::oneshot` per request plus a map keyed by `requestId`, and a
timeout that answers "no" rather than hanging the turn.

## Consequences

- One binary keeps serving its own UI, which is the property that made `webrain
  mcp --http` worth having.
- The static page is a second surface to keep honest with the docs; the ADR's
  mapping table is the contract.
- A browser is now a trust boundary: the page may render model output and
  tool arguments that came from a hostile page. Whitelist rendering, no
  `innerHTML`, and CSP `default-src 'self'`.
- Not doing B/C/D now means a user who wants a native app still cannot have one;
  A is the smallest step that removes the "no interface" complaint.

## Open questions

- Should `/turn` accept a *new* turn while one is streaming (queue) or refuse
  (409)? The CLI refuses; the UI probably wants a queue.
- Does the page need the token/cost block that `chat` prints, or is the meter
  enough? Cost is `n/a` on BYOK, so a meter that shows AIU would be misleading.
- Multi-session (`webrain chat --serve` with tabs) needs `Client::prepare_session`
  per tab and a session id in every request — deliberately out of scope for PR 1.
