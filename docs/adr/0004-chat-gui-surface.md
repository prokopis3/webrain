# ADR-004: a GUI for `webrain chat`

- Status: Accepted — implemented in `webrain-cli/src/chat_server.rs`
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

## Research: is there a frontend to reuse? (2026-11-04)

Checked before writing any HTML, because a ready UI would have beaten option A.

**The SDK ships no frontend.** In `github/copilot-sdk` the runnable examples are
terminal programs (`rust/examples/chat.rs` prints deltas and fills the
user-input slot with a `StdinUserInputHandler`; `nodejs/examples/basic-example.ts`
logs events). The web-facing documentation is backend-only —
`docs/setup/backend-services.md` (Express + `sendAndWait`, `RuntimeConnection.forUri`)
and `docs/setup/scaling.md` (session resume, `deleteSession`) — i.e. it tells you
how to put *your* UI on top, and stops there. So "adopt the vendor's chat UI" is
not an option that exists.

**AG-UI is the closest thing to a standard UI contract.** The Agent User
Interaction Protocol (`docs.ag-ui.com`) is a streaming event vocabulary over SSE,
and CopilotKit renders it with maintained React components. Its events map onto
what `session.subscribe()` already delivers:

| SDK event | AG-UI wire event |
|---|---|
| first `assistant.message_delta` of a turn | `RUN_STARTED` + `TEXT_MESSAGE_START` |
| `assistant.message_delta` | `TEXT_MESSAGE_CONTENT` (`delta`) |
| `assistant.message` | `TEXT_MESSAGE_END` |
| `tool.execution_start` | `TOOL_CALL_START` (`toolCallName`) |
| `tool.execution_complete` | `TOOL_CALL_END` + `TOOL_CALL_RESULT` |
| `session.usage_info` / `session.usage_checkpoint` | `CUSTOM` (`name: "usage"`) |
| `permission.requested` | `CUSTOM` (`name: "permission"`) — AG-UI's interrupt model is the general form, out of scope here |
| `model.call_failure` / `session.error` | `RUN_ERROR` |
| `assistant.idle` / `session.idle` | `RUN_FINISHED` |

**Decision, amended:** `/events` uses those names. Same bytes, same SSE framing,
same one-file page — but a CopilotKit (or any AG-UI) client can be pointed at the
endpoint later without the server changing, and `--serve`'s output stops being a
private format. We do **not** add a Node build to get CopilotKit now: it would
put a second toolchain in a repo whose selling point is one static binary, and
the page it would replace is ~300 lines.

**Why not a Rust GUI crate.** All four were considered and dropped:

| Crate | Why it loses here |
|---|---|
| `eframe`/`egui` | immediate-mode, no HTML: markdown, tables and images from model output are re-implemented by hand, and the binary gains a windowing stack (winit + wgpu/glow) that is useless on the headless boxes and containers this server usually runs on |
| `iced` | same windowing cost, plus an async message architecture that duplicates the event loop the SDK already provides |
| `slint` | needs the `.slint` compiler in the build and its own runtime; a UI language bought for one screen |
| `tauri` | correct for a desktop product, but it is a second binary + a web frontend build, i.e. strictly more than A for the same browser UI |

The pattern worth copying is the one `llama.cpp` uses for `llama-server`: the
binary serves its own page. That is A.

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
