# Chat SDK gaps — what `webrain chat` does not use yet

**Started as research only; the Tier 0/1 work, Plan steps 1–8 and 11, and the
2026-11-15 second pass (rewind, session mode, queue edit, four read routes,
three events) have since shipped.** Inventoried 2026-11-07 against
`github-copilot-sdk` 1.0.28, re-inventoried 2026-11-15 by three researcher passes
(config, RPC, events) plus the SDK's own repo and e2e suite, and implemented
2026-11-15. Rows that shipped say so, with the verification that was actually run.

How the numbers were obtained: the installed crate's `types.rs`, `lib.rs`,
`generated/rpc.rs`, `generated/session_events.rs` and `generated/api_types.rs`,
compared field-by-field with `webrain-cli/src/chat_session.rs` (`fn connect` is
the only `SessionConfig`/`ClientOptions` builder), the `rpc()` call sites in
`chat_server.rs`, and the `handle()` switch in `webrain-cli/ui/src/App.svelte`.

## The totals

| Surface | SDK offers | We use (2026-11-07) | We use (2026-11-15) | Still open |
|---|---|---|---|---|
| `SessionConfig` fields | 99 | 17 | 21 (+`large_output` dir, MCP `timeout`, `file_change_tracking`, `reasoning_summary`, `name`) | 78 |
| `ClientOptions` fields | 29 | 5 | 5 | 24 |
| `SessionRpc` namespaces | 47 | 9 | 18 (+`permissions`, `tools`, `queue`, `options`, `history`, `mode`, `instructions`, `quota`, `limitPrediction`, `metadata`, `ui`) | 29 |
| Session events | 158 | 33 | 45 named `case`s (50 labels − 5 `webrain.*` synthesized), plus `assistant.reasoning`/`_delta` via the fallback | 113 |

`spawn_live` (`chat_server.rs:184`) forwards every event to the SSE bus
unchanged, so all 158 already reach the browser. Every frontend item below is a
`case` in `handle()`, not new plumbing. `ClientRpc` (34 namespaces) is called
**zero** times — everything we do is session-level.

## Shipped 2026-11-15

One pass, in the order it was built, with what was actually verified rather than
what the code was intended to do:

| # | what | verified by |
|---|---|---|
| 1 | `large_output.output_directory` → `<cwd>/.webrain/tool-output` | the dir is created at connect and gitignored; a big result no longer spills to a fenced-off OS temp path |
| 2 | MCP server `timeout: 600 s` | long crawls stop being reported as failed tools |
| 3 | `organization_custom_instructions` (standing instructions) | round-trips through `POST /settings` |
| 4 | 11 previously-ignored events (`assistant.intent`, `agent.interrupted`, `turn_retry`, `tool_call_delta`, `title_changed`, `context_cleared`, `context_changed`, …) | each renders where it was listed in Tier 4 |
| 5 | `POST /tool` + "edit & re-run" on a tool row | `POST /tool {"name":"webrain-webrain_stats"}` returned the tool's real result |
| 6 | `GET/POST /permission-mode` + Runtime-tab control | the runtime emitted `permissions_changed manual → assisted → manual` |
| 7 | `GET /pending` — approvals survive a refresh | `{"asks":[]}` on a quiet server; the payload is the same card the SSE event carried |
| 8 | `POST /skills`, `POST /mcp` + togglable cards | the row flipped on→off→on and `session.skills_loaded` came back |
| 9 | `GET/POST /queue` + a rail above the composer | `{"items":[],"steering":[],"withdrawable":null}` on a quiet server |
| 10 | `session.options.update` — a settings save no longer reseeds | `restart:false, patched:{ok:true}` for instructions; `restart:true` for `max_output_tokens` |
| 11 | MCP health: `session.mcp_server_status_changed` / `_tools_loaded` / `_needs_reconnect` / `_removed` | verified live — `POST /mcp enabled:false` produced the banner "tool server webrain · disabled" with a reconnect button, and clicking it cleared the banner when the status went back to `connected` |
| 12 | Memory writes as their own approval card | the card now carries the fact, its subject and its citations (`permission_card` unit test: `a_memory_write_asks_about_the_fact_not_the_tool`); no live click-through — that needs a model turn |

Verified-live means the route was called against a running `webrain chat --serve`
and the answer is quoted above. Two UI paths could not be exercised end-to-end in
that session because the provider account was out of credits (HTTP 402), so no
model turn could start: the **edit & re-run button** and the **queue rail** are
compiled and their routes are verified, but neither has been clicked through
against a live turn. Treat that as the first thing to re-check.

## Corrections to the first inventory

Three claims in the 2026-11-07 pass were wrong. Recording them so the plan is not
built on them:

1. **"19 events handled" was an undercount.** `handle()` covers **33** of 158:
   37 `case` labels, minus `tool.execution_end` (which is *not a catalog event* —
   a dead alias), minus 5 server-synthesised `webrain.*` types, **plus** 2
   (`assistant.reasoning`, `assistant.reasoning_delta`) that are caught only by
   the `/reason|think|thought/i` fallback in the `default` arm. Today's reasoning
   panel reads the fallback, not a named case.
2. ~~**Tier 1 #2 was only half shipped.** `tool.execution_progress` has a case;
   **`assistant.tool_call_delta` does not** — there is no `case` for it anywhere
   in `handle()`.~~ **Superseded 2026-11-15**: both have a `case` now
   (`App.svelte:926` is `tool_call_delta`), and it drives "asking for <tool>"
   while the arguments stream.
3. **`tool.execution_end` is not real.** The page has an arm for it, which can
   never fire; the event that settles a tool row is `tool.execution_complete`.
   *(The arm was deleted 2026-11-15 — nothing to fix, kept as a warning against
   re-adding it.)*

## Tier 0 — a bug, one line — **FIXED 2026-11-07**

`enable_skills` was never set, and `ClientMode::Empty` defaults it to `false`
(`session.rs:2189-2190`), so `with_skill_directories` was inert while the `@`
menu listed those skills. Fixed with one line; verified live (the agent calls
`skill` and loads `webrain`; first request 3 186 → 3 600 tokens).

## Tier 1 — visible in the chat, cheap — **SHIPPED 2026-11-07**

Twelve events that were already reaching the browser and being thrown away:
warnings/info/notifications, `tool.execution_progress`, compaction and
truncation, `plan_changed`, `model_change`/`mode_changed`, sub-agent completion
numbers, `prompt_cache_break`, `skill.invoked`, `tool_search.activated`,
`permission.completed`, `abort`.

Two rows were still open and are carried into Tier 4 below:
`assistant.tool_call_delta` (never handled) and the `tool.execution_end` arm
(dead — delete it). **Both closed 2026-11-15** — the `case` exists, the dead arm
is gone.

## Tier 2 — config we do not set

Every field reachable from `SessionConfig` except the ones in "Refuse". Sorted by
value ÷ work. "BYOK" means it has an effect without a Copilot subscription.

| # | field | source | effect | backend | frontend | work |
|---|---|---|---|---|---|---|
| 1 | ~~`large_output.output_directory`~~ | `types.rs:2403`, type `864-884` | **SHIPPED 2026-11-15.** Behaviour was already on (CLI default: enabled, 50 KB); the gap was *where*. A big extraction spilled to OS temp, which our own path fence (`chat_tools.rs:33`, rule at `chat_session.rs:70-77`) then refused — the model got a preview plus an unreadable path and re-scraped. Now `<cwd>/.webrain/tool-output`, created at connect and gitignored | 3 lines in `connect` | — | S |
| 2 | ~~`McpStdioServerConfig.timeout`~~ / `env` / `tools` | `types.rs:1330-1360` | **SHIPPED 2026-11-15** for `timeout: Some(600_000)`: the runtime's default governed a crawl that legitimately runs minutes, and a killed call reports as a failed tool. `env`/`tools` still unset | 2–4 lines | — | S |
| 3 | ~~`organization_custom_instructions`~~ | `types.rs:2317` | **SHIPPED 2026-11-15** — `RuntimeSettings.instructions`, a textarea in the Runtime tab, applied by the runtime instead of pasted onto every prompt (no per-turn tokens, survives a restart). Verified: `POST /settings` returns `restart: true` and the value round-trips | `RuntimeSettings` field | textarea in the Runtime tab | S |
| 4 | `ask_user_variant` + `ElicitationHandler` | `types.rs:2245`, `2240-2243` | Today `ask_user` is one free-text box (`WebInput`, `chat_server.rs:96`); Elicitation lets the model send a structured question the page can render as buttons — the fidelity our approval card already has | second handler beside `WebInput` | choice card, same shape as `askcard` | M |
| 5 | `name` (+ `session_id`) | `types.rs:2218`, `2211-2216` | "A name also makes the session persist from the start, so it can be listed and resumed even if it never runs a turn." This is the field that unlocks `ResumeSessionConfig` — and the SDK's own e2e proves resume works **from a new client**: `rust/tests/e2e/session.rs::should_resume_a_session_using_a_new_client` (`resume_session(...with_continue_pending_work(true))`) | name at create + a resume route | history menu stops being read-only | L |
| 6 | `session_limits` | `types.rs:2474` | **Experimental.** A hard cap on a runaway turn — the page prices every turn already (`turn_cost`, `chat_server.rs:693`) but nothing stops one | settings key | number input | M |
| 7 | `custom_agents` + `default_agent` + `agent` | `types.rs:2426/2430/2433`, type `713-800` | Agents with their own prompt, tool allowlist, **model** and reasoning effort. Turns `--allow-agents` into a picker, and per-agent `model` is how a sub-agent gets the cheap model without rebuilding the session | build from skill/agent dirs | composer agent picker | L |
| 8 | `providers` + `models` | `types.rs:2453/2459`, `1690`, `1855` | **Experimental.** Named BYOK connections selectable as `provider/id` inside one session — `/switch` to another endpoint would stop rebuilding (`chat_server.rs:1897`), and `ProviderModelConfig::max_context_window_tokens` replaces the `max_prompt_tokens` hack | replace the single-provider path | menus stop reloading | L |
| 9 | `diagnostics` | `types.rs:2288` | **BLOCKED in 1.0.28 — not S, not ours to build.** The config exists only to *ask* for the stream: `SessionRpcDiagnostics` exposes `configure()` and nothing else, and the event catalog has no diagnostic report, so there is no read path once the flag is set. Setting it would add a capability we cannot observe. Re-check after an SDK bump: the day a diagnostic event (or a `SessionRpcDiagnostics::get`-style call) shows up in `types.rs`, this row is S again — flag + bus drain | flag + bus drain | diagnostics tab in the `/mcp` drawer | **blocked** |
| 10 | ~~`enable_file_change_tracking`~~ | `types.rs:2472` | **SHIPPED 2026-11-15** — set for `--serve`, and it is what makes the files half of `session.history.rewind` possible at all (`GET /rewind` answered `fileChangeTrackingEnabled: true` on the first live call). Without it "undo" was the conversation only | 1 line in `connect` | the `/rewind` card | **DONE** |
| 11 | `system_message_transform` | `types.rs:2605` | Per-turn injection (current CDP target, active profile, the page on screen) without a session rebuild | implement the handler | — | M |
| 12 | ~~`reasoning_summary`~~ | `types.rs:2233` | **SHIPPED 2026-11-15** — `Concise`, set whenever an effort is set. The page has a thinking panel, and what a model summarises is otherwise the runtime's call, so a provider that can emit summaries stayed silent | 1 line | existing thinking panel | **DONE** |

**Not gaps — do not "fix" these:** `large_output` enabled (default on);
`refresh_custom_instructions` (targets a discovery cache we switch off);
`skip_custom_instructions` and `custom_agents_local_only` (already the
`ClientMode::Empty` defaults, and what we want); `event_buffer_capacity` (we drain
continuously); `context_tier` and `capi` (Copilot-routing concepts, BYOK has one
context size); `trust_working_directory` (our own `PermissionHandler` owns this);
`image_generation` (doc says BYOK is unsupported).

## Tier 3 — RPC namespaces we do not call

38 of 47 session namespaces were uncalled. The ones that buy a chat capability:

| # | namespace | source | what it buys | surface | work |
|---|---|---|---|---|---|
| 1 | ~~`session.tools.execute`~~ | `rpc.rs:9014` (`getBuiltinDescriptors:18575`) | **SHIPPED 2026-11-15** — "Executes one tool from the session's currently offered tool set through the native invocation pipeline". `POST /tool`, verified live against `webrain-webrain_stats`; the page grows "edit & re-run" on the tool row | "edit & re-run" on the tool row + `POST /tool` | **DONE** |
| 2 | ~~`session.history.*`~~ | `rpc.rs` — `listRewindPoints:11787`, `previewRewind:11819`, `rewind:11855` | **SHIPPED 2026-11-15** — a real conversation undo, where `/undo` only restores files and `/fork` only copies a transcript. `/rewind` lists the points, picks one, and for the file half shows what it would restore before writing. The `event_id` problem in the old plan is gone: `listRewindPoints` *hands back* the ids, so the page never had to keep them. Verified live: `{"files":true,"unavailable":"session-busy","points":[]}` mid-turn (the SDK's documented transient state), and a bad id answers `Rewind event 'nope' is not a current root user turn` | `/rewind` command + a `pick` card | **DONE** |
| 3 | ~~`session.permissions.setMode/getMode/pendingRequests`~~ | `rpc.rs:14890`, `setMode:14868`, `getMode:14897` | **SHIPPED 2026-11-15** — one control (`manual` / `assisted` / `allow-all`) in the Runtime tab, verified live against the runtime's own `permissions_changed` event. The orphaned-sender defect (a page refresh killed the turn) is fixed with our own store: `WebPermission` keeps the card beside its sender and `GET /pending` redraws it on reload | settings card | **DONE** |
| 4 | ~~`session.mcp.enable/disable/restartServer`, `session.skills.enable/disable`~~ | `rpc.rs:8867`, `8993` | **SHIPPED 2026-11-15** — `/mcp` and `/skills` became togglable cards (verified live: the row flipped and the runtime echoed `session.skills_loaded`). "The tool set is fixed at session creation" stops being true for MCP- and skill-provided tools | toggles on the existing drawers | **DONE** |
| 5 | ~~`session.queue.pendingItems/sendNow/removeAt/updateText`~~ | `rpc.rs:8944` | **SHIPPED 2026-11-15, all four calls.** `pendingItems` + `sendNow` gave `GET/POST /queue` and the rail; `updateText` + `removeAt` gave the rail **edit** and **drop** buttons, which the first pass left out. A prompt you can send but not correct is one you retype in the composer — a second turn instead of a fixed first one | the rail above the composer | **DONE** |
| 6 | ~~`session.mode.set/get`~~ | `rpc.rs:8881` | **SHIPPED 2026-11-15** — the composer's mode stops being a sentence built by `with_mode` and becomes the mode the agent operates in. `GET/POST /mode` (`interactive`/`plan`/`autopilot`), and every turn posts it. Verified live: `POST /mode {"mode":"plan"}` → `GET /mode` → `"plan"` → back to `"interactive"`. The mapping is unit-tested, including that a typo can never land in `autopilot` | existing mode control | **DONE** |
| 7 | ~~`session.ui.ephemeralQuery`~~ | `rpc.rs:18870` | **SHIPPED 2026-11-15** — `POST /ephemeral {"question":…}` plus a **summarise** button on every tool row that has a result. "Runs a transient no-tools model query against the current conversation context": the model cannot act on the answer and the exchange never joins the transcript, so asking about a 200 KB extraction costs one call instead of one more turn. The route is exercised (a bad body answers `{"error":"question"}`); the button itself was not clicked in that session | `POST /ephemeral` + the tool row | **DONE** (route + button) |
| 8 | ~~`session.metadata.isProcessing`~~ | `rpc.rs:8874` | **SHIPPED 2026-11-15** — `GET /sessions` now carries `processing` from the runtime, falling back to our `AtomicBool` only when the call fails. That flag could be left stuck on by a page reload and was never set by a background continuation. Verified live: `processing: true` while a turn ran | status chip | **DONE** |
| 9 | ~~`session.instructions.getSources`~~ | `rpc.rs:8839` | **SHIPPED 2026-11-15** — `/instructions` lists what the model was told before the user's words, in merge order, without the file bodies. Verified live against this repo: `.github/copilot-instructions.md`, `AGENTS.md`, `mermaid.instructions.md` | `/instructions` command | **DONE** |
| 10 | ~~`session.options.update`~~ | `rpc.rs:8902` | **SHIPPED 2026-11-15** — the fix that stops a settings save from costing a prompt cache. `merge` now compares only `memory`, `allow_agents`, `max_output_tokens`, `compact_at`; `session_store`, `effort` and `instructions` are patched into the live session. Verified live: instructions → `restart:false, patched:{ok:true}`, `max_output_tokens` → `restart:true` | `POST /settings` stops forcing a reseed | **DONE** |
| 11 | ~~`session.tasks.cancel/getProgress/readOutput`~~ | — | **NOT IN 1.0.28.** `SessionRpcTasks` has exactly two methods, `startAgent` and `list`; the three names in the 2026-11-07 inventory do not exist, so a per-agent stop button needs an SDK bump rather than wiring | — | blocked |
| 12 | ~~`session.quota.get`, `session.limitPrediction.predict`~~ | `rpc.rs:8951`, `17081` | **SHIPPED 2026-11-15** — `/quota`. Verified live: it answered a real projection (`planTier: unknown`, a 459/1877/4573/29931 cap ladder, `recommendedTier: recommended`) rather than null, so the "null on BYOK" assumption in the old plan was wrong — the runtime reports a plan even on a key of your own | `/quota` command | **DONE** |

Also uncalled, lower value: `session.agent.*` (needs #7 above first),
`session.history.compact/truncate` (the runtime already compacts),
`session.provider.add` (superseded by `providers`/`models`), `session.eventLog.*`
(we keep our own transcript), `session.workspaces.*` (our upload route covers
it), `session.providers.getCatalog/discover` (a "probe this key" button),
`session.name.set` (stored by the runtime *and* our store — pick one),
`session.debug.collectLogs` (a download button), `session.shell.*` (off-mission).

## Tier 4 — events still ignored

125 of 158 were unhandled at the first inventory; three more landed in the second
pass. Every event below was already in the browser, and each one shows the user
something they could not see:

| event | source | what the user cannot see | attaches to | work |
|---|---|---|---|---|
| ~~`assistant.intent`~~ | `session_events.rs:166` | **SHIPPED 2026-11-15** — the agent's own one-line statement replaces "working" in `liveStep` | `liveStep` | S |
| ~~`agent.interrupted`~~ | `:164` | **SHIPPED 2026-11-15** — what was running and for how long, where `abort` only prints our reason | note | S |
| ~~`assistant.turn_retry`~~ | `:162` | **SHIPPED 2026-11-15** — a retried model call, with the provider's reason | note | S |
| ~~`session.title_changed`~~ | `:26` | **SHIPPED 2026-11-15** — reloads the rail instead of keeping the stale title | `convos` row | S |
| ~~`session.context_cleared`~~ | `:101` | **SHIPPED 2026-11-15** — note plus a reset ctx meter | note + meter | S |
| ~~`session.context_changed`~~ | `:97` | **SHIPPED 2026-11-15** — the working directory, once per turn | note | S |
| ~~`assistant.tool_call_delta`~~ | `:210` | **SHIPPED 2026-11-15** — "asking for <tool>" while the arguments stream | `liveStep` | S |
| ~~`session.shutdown`~~ | `:84` | **SHIPPED 2026-11-15** — the session's bill (tokens, cost per model, files changed and lines) is a `usage` row, and the shutdown reason is kept. It used to be one sentence that threw the numbers away | usage row | **DONE** |
| ~~`pending_messages.modified`~~ | `:158` | **SHIPPED 2026-11-15** — signal-only, as documented: the payload is `{}`, so the arm reloads the queue rail. Without it a prompt queued by something other than this click leaves the rail showing a list the runtime no longer has | `loadQueue()` | **DONE** |
| ~~`user.message`~~ | `:156` | **SHIPPED 2026-11-15** — a message that did not come from this composer (an autopilot continuation, another client's steering note) now draws a bubble. The page's own sends are recognised by the first 80 characters it drew, compared as a substring because the prompt reaches the runtime with a mode sentence on the front and controls on the back | transcript bubble | **DONE** |

**MCP health — SHIPPED 2026-11-15.** The trio
`session.mcp_server_status_changed` (`:510`), `session.mcp_server_tools_loaded`
(`:512`) and `session.mcp_server_needs_reconnect` (`:516`) used to collapse into
the `act` row, so a dead tool server looked like a model with no tools. They now
feed a per-server map: any status that is not `connected`/`pending` raises a
banner naming the server and its error, with a **reconnect** button that posts
`/mcp` and waits for the status event to clear it. `session.mcp_server_removed`
is handled too.

Noise here (do not handle): session start/resume handshakes, autopilot,
scheduler, quota/auto-tier (Copilot-only), workspace-file and snapshot events,
`fusion_*` / `assistant.fusion_*` (HydraFusion routing), canvas/extension/plugin/
MCP-app events, managed-settings, sandbox, accounts/OAuth, hook start/end, the
`*_ref` twins of `skill.*`, `model.call_*` (folded into `activity()` on purpose),
`assistant.message_start`, `assistant.streaming_delta`, `tool.shell_output`.

## Refuse

Canvases, extensions, connectors, plugins/marketplaces, fusion, autopilot,
remote/cloud sessions, managed settings, sandbox, accounts/OAuth login,
image generation. Each is enterprise, a different product, or needs a renderer
we do not ship — and every one of them is surface that has to be fenced.

Note: **`session.fusion` does not exist** in 1.0.28. A plan that names it is
stale.

## Plan — in the order I would build it

Cheap and user-visible first; each row is one route plus one element.

1. **`large_output.output_directory`** (S) — **DONE 2026-11-15**.
2. **`McpStdioServerConfig.timeout`** (S) — **DONE 2026-11-15** (600 s).
3. **`assistant.intent` + `agent.interrupted` + `assistant.turn_retry`** (S each)
   — **DONE 2026-11-15**: three `case`s plus `assistant.tool_call_delta`, so "working"
   became what is actually happening.
4. **`session.title_changed` + `session.context_cleared` + `session.context_changed`**
   — **DONE 2026-11-15**: the sidebar and the ctx meter stop lying. The dead
   `tool.execution_end` arm was deleted in the same pass.
5. **`session.permissions.setMode` + `pendingRequests`** (M) — **DONE
   2026-11-15**: `GET/POST /permission-mode` plus a segmented control in the
   Runtime tab (verified live: the runtime emitted
   `permissions_changed "manual" → "assisted" → "manual"`). The orphaned-sender
   defect is fixed a cheaper way than the SDK call: `WebPermission` keeps the
   card payload beside its `oneshot` sender, and `GET /pending` hands the open
   requests back to a reloaded page, which redraws the cards.
6. **`session.tools.execute`** (M) — **DONE 2026-11-15**: `POST /tool`, verified
   live against `webrain-webrain_stats`. The page grows "edit & re-run" on a
   tool row: the arguments become a textarea and the reply lands as a new row, so
   the model's original call stays visible. Note the name split — the wire name is
   `webrain-webrain_navigate`, so a re-run must send that one back **unmodified**.
7. **`session.mcp.enable/disable` + `session.skills.enable/disable`** (S) — **DONE
   2026-11-15**: `/mcp` and `/skills` became togglable cards instead of notes
   (verified live: row flipped on→off→on, and the runtime echoed
   `session.skills_loaded`).
8. **`organization_custom_instructions`** (S) — **DONE 2026-11-15** (the Runtime
   tab's "standing instructions").
9. **MCP-health events** (S, after reading their payload) — a dead tool server
   must not look like a model with no tools. **DONE** — and the events were not
   in the catalogue where this row expected them: they are
   `session.mcp_server_status_changed` / `_tools_loaded` / `_needs_reconnect` /
   `_removed`, all four handled (`App.svelte:1282-1307`) into an `mcpHealth` map
   that renders the "N tool server(s) down" banner above the composer. `GET /mcp`
   still answers the same question on reload (`servers[].status`), so the banner
   survives a refresh.
10. **`session.history.listRewindPoints` / `previewRewind` / `rewind`** (M) —
    **DONE 2026-11-15**: `/rewind` plus a `pick` card, and
    `enable_file_change_tracking` with it. The old note said the page would have
    to "plumb ids first" — it does not: the points list *is* the id list.
11. **`session.queue.*`** (S/M) — **DONE 2026-11-15, all four calls**:
    `pendingItems` + `sendNow` gave `GET/POST /queue` and the rail;
    `updateText` + `removeAt` gave it edit and drop.
12. **`name` + `ResumeSessionConfig`** (L) — resume instead of replay-a-transcript.
    The SDK's e2e proves cross-client resume works; the cost is re-supplying the
    whole callback set (`permission_handler`, `hooks`, `mcp_servers`, `memory`,
    dirs) or the resumed session silently differs. **Still open.**
13. **`session.options.update`** (M) — **DONE 2026-11-15**, and it is the reason a
    settings save no longer costs a prompt cache. `RuntimeSettings::merge` now
    compares only the four fields a live session cannot absorb
    (`memory`, `allow_agents`, `max_output_tokens`, `compact_at`);
    `session_store`, `effort` and `instructions` are patched into the running
    session instead. Verified live: `POST /settings {"instructions": …}` answers
    `restart: false, patched: {ok: true}`, while `max_output_tokens` still answers
    `restart: true`.

**Verify before building #12:** re-supplied-field behaviour on resume is the whole
risk. It is answered by reading the crate, not by guessing — and until then a
resumed session that silently lost its MCP server would look like a model that
simply has no tools.

## Memory across sessions — what is actually ours to change 2026-11-15

The runtime's memory store is **not addressable**: there is no `session.memory.*`
namespace in 1.0.28, no memory event, and no config beyond
`MemoryConfiguration { enabled, mode }`. What it *does* expose is the write:
every stored fact arrives as a permission request of `kind: "memory"` with
`{ action: store|vote, fact, subject, citations, direction, reason }`.

That is the lever, and it was wasted: those requests fell through the generic
tool branch of `WebPermission::handle`, so the one card a reader must be able to
judge arrived as a tool named `tool` with no arguments. They now render as their
own card — the fact quoted, its subject and citations under it, Deny / Remember /
Always remember. "Always remember" uses the same `autoTools` key as any other
approval, so it is a per-tab policy, not a hidden global.

Still open on purpose: `MemoryConfiguration.mode` (`system-prompt` vs
`user-prompt`) is not exposed, and memory stays **off** by default
(`RuntimeSettings::default().memory == false`). Both are one-line changes; both
change what a turn costs or writes without the user asking, so they are a product
decision, not a gap.

Still open from the M/L tier, and deliberately not attempted: #12 resume (it needs
the whole callback set re-supplied or the resumed session silently differs), Tier 2
#4 elicitation (it would replace a working free-text `ask_user` box with buttons —
a fidelity change, not a capability the page lacks), Tier 2 #7 custom agents (we
run with `enable_config_discovery = false`, so a picker built on disk discovery
would contradict the session we configured), Tier 2 #8 the providers registry, and
Tier 2 #9 `diagnostics` (the config is two nested structs; the work is the drawer
that would show the entries). Tier 2 #6 `session_limits` was read and dropped: its
one field is `max_ai_credits`, a Copilot-plan budget, not a cap on a runaway turn.

## Second pass, 2026-11-15 (after the first one was written up)

| what | route or config | verified by |
|---|---|---|
| rewind points | `GET /rewind` | live mid-turn: `{"files":true,"unavailable":"session-busy","points":[]}` |
| rewind preview + rewind | `POST /rewind` | bad id → `Rewind event 'nope' is not a current root user turn`; the busy path is the SDK's `session-busy` |
| file change tracking | `enable_file_change_tracking` for `--serve` | the same call answered `fileChangeTrackingEnabled: true` |
| session mode | `GET/POST /mode` | live: `plan` → `GET /mode` → `plan` → back to `interactive`; mapping unit-tested |
| instruction sources | `GET /instructions` | live: the three files this repo actually loads |
| credits | `GET /quota` | live: a real cap ladder, not null |
| queue edit / drop | `POST /queue {"text"}` / `{"remove":true}` | live: unknown id → `unknown queue item id 'nope'` |
| busy truth | `metadata.isProcessing` in `GET /sessions` | live: `processing: true` while a turn ran |
| three events | `user.message`, `pending_messages.modified`, `session.shutdown` | `user.message`: a live turn produced **one** bubble, not the page's plus the runtime's; `pending_messages.modified` and `session.shutdown` are compiled but were not observed (no queue and no shutdown in that session) |
| transient query | `POST /ephemeral` | bad body → `{"error":"question"}`; the tool-row **summarise** button is compiled but was not clicked |

**The `pick` card was clicked, and it was broken.** Rows carried a `run` closure;
Svelte's state proxy handed the button a row without one, so the first click threw
`TypeError: e(...).run is not a function`. The rows now carry plain data and
`runPick(r)` dispatches on `act`, the same shape the `list` card has used all along
(`flip(item, r)`). After the fix, live: `/rewind` → card → click → **`rewound · 8
events dropped`**, and the turn and the card left the transcript. The `…and files`
button is only offered for a turn that actually captured files (`p.files > 0`), not
merely when tracking is on — so the file-restore half is still **unproven**: it
needs a turn that edited a file.

Also verified by eye during the same live turn: the reasoning block sits above the
call or answer it explains (`thinking` → tool → `thinking` → answer), which is the
per-step filing from the previous pass holding up under a real 300-line turn.

## Third pass, 2026-11-20 — what the totals still hide



The numbers in "The totals" were counted by hand and one of them was wrong in the

direction that matters: **a call chain broken across lines hides a namespace.**

`POST /tool` is written as `.rpc()` then `.tools().execute(...)`, so a grep for

`.rpc().tools(` found nothing and `tools` was carried as "used but unmeasured".

Re-counted here by looking for `.<namespace>()` anywhere in `webrain-cli/src`:



| SessionRpc namespaces | count |

|---|---|

| offered by 1.0.28 | 47 |

| mentioned in the CLI | 19 (`fleet`, `history`, `instructions`, `limit_prediction`, `mcp`, `metadata`, `mode`, `model`, `options`, `permissions`, `plan`, `providers`, `queue`, `quota`, `skills`, `tasks`, `tools`, `ui`, `usage`) |

| **no call site at all** | **28** |



The 28, and why each is where it is:



| namespace | methods (read from `rpc.rs`) | verdict |

|---|---|---|

| `completions` | `get_trigger_characters`, `request` | **PROBED LIVE 2026-11-21 — absent here.** `GET /completions` on a running `chat --serve` answered `{"trigger_characters":[],"supported":false}`, which is the SDK's documented way of saying a **local session does not advertise host-driven completions**. So the route stays as a probe (it costs one call and re-answers after an SDK bump), and no ghost-text UI was built: it would be interface over an empty list. If a future build answers non-empty, the contract is recorded — `request { offset, text }` in **UTF-16 code units**, back `items[]` of `{ insert_text, label?, kind?, range_start?, range_end? }`, accepting replaces `[range_start, range_end)` or the token at the cursor |

| `name` | `get`, `set`, `set_auto` | the RPC half of Tier 2 #5 — `set` is what makes a session persist *before* its first turn, so it is a prerequisite for resume (#12), not a feature of its own |

| `settings` | `snapshot`, `evaluatePredicate` | **not ours to call** — both are `pub(crate)` in 1.0.28, so the SDK publishes no way in and there is nothing to wire. Closed 2026-11-21 by reading `rpc.rs:17657` |

| `visibility` | `get`, `set` | **Refuse** — `visibility.get` reports the Mission Control sharing status (`repo` / `unshared`) and a shareable GitHub URL, i.e. GitHub cloud session sharing, not a chat feature. Closed 2026-11-21 |

| `lsp` | not read | a language server inside the session; the agent already has `grep`/`view` tools, so the value is unclear |

| `debug`, `event_log`, `telemetry` | — | diagnostics-shaped, and the page cannot read them anyway (see Tier 2 #9) |

| `canvas`, `connectors`, `extensions`, `plugins`, `managed_settings`, `sandbox`, `remote`, `schedule`, `workflow`, `workspaces`, `autopilot_objective`, `agent`, `accounts`, `git_hub_auth`, `content_exclusion`, `customizations`, `provider`, `shell`, `commands` | — | in "Refuse" already: enterprise, a different product, or a renderer we do not ship |



`session.tools` is used, but only its `execute`. The rest of that namespace is

worth naming because two methods answer a question the page currently guesses at:



| method | what it would change |

|---|---|

| `list_pending_requests`, `handle_pending_tool_call` | **SHIPPED 2026-11-21** as a probe plus an answer path. `GET /pending` now also reports what the *runtime* is holding — `runtime.external` from `tools.listPendingRequests` and `runtime.permissions` from `permissions.pendingRequests` — beside our own `asks`, kept in a separate key so a renderer that does not know them cannot break. `POST /pending {request_id, result|error}` answers an external call through `handlePendingToolCall`. Live: `{"asks":[],"runtime":{"external":[],"permissions":[]}}` on a quiet session, `400 {"error":"request_id"}` without an id, and `{"handled":false}` for an unknown one. Both lists are empty *by construction* here — we register no external tools, and our `PermissionHandler` answers prompts in-process — so this is insurance against an observed-but-unexplained wait, not a fix for a reproduced bug |

| `get_current_metadata`, `initialize_and_validate` | **neither exists in 1.0.28** — this row was written from the plan, not from `rpc.rs`. `SessionRpcTools` has exactly `execute`, `getBuiltinDescriptors`, `taskCompleteEventData`, `handlePendingToolCall`, `listPendingRequests`; all five now have a call site or a reason not to |



### ClientRpc — 34 namespaces, one call



Everything webrain chat does is session-level, which is defensible (one process,

one conversation) and is also why these are all still open. Listed so a later

"why can't the page do X" has somewhere to start:



`account`, `agent_registry`, `agents`, `azure_ml`, `catalog`, `commands`,

`connectors`, `content_exclusion`, `environments`, `extensions`, `feature_flags`,

`git`, `git_hub_owners`, `git_hub_repository`, `global_state`, `hooks`, `host`,

`instructions`, `llm_inference`, `managed_settings`, `mcp`, `models`,

`permissions`, `persistence`, `plugins`, `repo`, `runtime`, `sandbox`, `secrets`,

`session_fs`, `sessions`, `skills`, `tools`, `user`



Three of them are the interesting ones for a chat client, and none is a chat

feature:



- **`sessions`** — create/list/delete a session from the client rather than letting our store own the list. `sessions.list` shipped 2026-11-21 (`GET /runtime/sessions`): the first time the page can see a thread the runtime holds and this machine's store does not. create/delete stay unbuilt, so resume is blocked on the *callback* half rather than on the list.

- **`runtime`**, **`host`** — version, capabilities and environment as the

  runtime reports them, instead of `--json` flags we parse ourselves.

- **`secrets`** / **`user`** — where a provider key would live if it lived in the

  runtime instead of our vault. Our vault is deliberate (it is the only store

  with TOTP for `webrain login`), so this stays refused.



Nothing here is scheduled. Both candidate rows from this pass were probed on
2026-11-21 and neither turned into a feature: `completions` answers an empty
trigger list on a local session (so there is nothing to complete), and the two
pending lists answer empty on a quiet session (so the "approval nobody was shown"
hole is theoretical in this deployment — the routes now report it rather than
close a reproduced bug).

That leaves **resume** (`name` + `ResumeSessionConfig`, with `ClientRpc.sessions`)
as the first gap that is still purely a capability: we replay a transcript where
the runtime could restore a session, and until then a thread the runtime still
holds cannot be offered for continuation. It stays L for the reason already
recorded — the callback set has to be re-supplied or the resumed session silently
differs — and it is the only row left worth a design pass.

## Fourth pass, 2026-11-21 — the two S rows and a name

Built after the third pass, and small on purpose: the L rows (resume, custom
agents, the providers registry) each need a design pass of their own, so this one
took what was a config field or a read.

| what | where | verified by |
|---|---|---|
| `SessionConfig.name` at create — `webrain-<convo>` | one line in `connect`; `spawn_live` passes the conversation id | `GET /name` on a running server answers `{"name":"webrain-c1795274324668"}` |
| `session.name.get` / `.set` | `GET /name`, `POST /name` | `{"saved":true,"name":"webrain-test-name"}`, then `GET /name` → the same; `400 {"error":"name"}` on an empty body |
| `ClientRpc.sessions.list` | `GET /runtime/sessions` | the runtime's own list: `summary`/`name` `webrain-c1795274324668`, `clientName sdk`, `context.cwd` — including sessions our store never wrote |

**Why `name` and not resume.** The name is what makes the runtime persist a
session from creation, and `sessions.list` is what a resume looks up. Both are now
in place and neither is resume: the remaining half is re-supplying the whole
callback set (`permission_handler`, hooks, MCP servers, memory, dirs) on
`ResumeSessionConfig`, or the resumed session silently differs from the saved one.
That stays the single row here that needs a design pass.

**Corrections in this pass.**

- Tier 3's `tools.get_current_metadata` / `initialize_and_validate` do not exist in
  1.0.28. `SessionRpcTools` is exactly `execute`, `getBuiltinDescriptors`,
  `taskCompleteEventData`, `handlePendingToolCall`, `listPendingRequests`.
- `ClientRpc.runtime` is not "version, capabilities and environment": it has one
  method, `shutdown`. That claim was written from the plan, not from `rpc.rs`.

## Fifth pass, 2026-11-21 — the baseline moved: `copilot-sdk` 2.0.1

Every number above was counted against `github-copilot-sdk` 1.0.28. That crate is
**deprecated**: its own 1.0.35 metadata says *"Migrate to copilot-sdk"*, and the
successor is `copilot-sdk` **2.0.1**. So this table is a 1.0.x table, and it is
the last one that can be produced this way.

**What can no longer be counted by reading.** In 2.0.1 only `types.rs` is
authored. `rpc.rs` and `session_events.rs` are 7-line facades over a private
`generated` module produced at build time, and no schema ships in the crate. The
"47 namespaces" and "158 events" rows above therefore **cannot be carried forward
by reading the crate** — they need a build, or the runtime's own schema.

### The build settles it: the 2.0.1 totals

`cargo check` writes the module the facades point at — 18 files, 3.6 MB, `rpc.rs`
alone 919 KB — so the numbers are countable again, just not before a build.
Counted 2026-11-21:

| what | 1.0.28 | 2.0.1 |
|---|---|---|
| session-config fields | 99, flat | 31, of them 19 groups |
| client-option fields | 29 | 13 |
| rpc namespaces | 47 | 44 client + 65 session = 109 |
| event types | 158 | 167 |

Every row moves in the same direction: the flat config surface shrank and the
reachable protocol surface grew. 47 → 109 namespaces is not a bigger API to
learn — the generator now nests, so `SessionRpcMcpConfig` is reachable where
1.0.28 made you spell the method name — but it does mean "we use 4 of 47" is no
longer the right way to describe what we leave on the table. That is the
migration's real cost: we are not losing settings, we are re-filing them.

`SessionConfig` — the 99-field object this whole document is measured against —
also no longer exists. What replaces it as a useful question is: *which group does
each setting live in, and which groups do we still leave empty*. The mapping is in
`docs/COPILOT_SDK_V2_MIGRATION.md`.

### Capabilities 2.0.1 adds that this document never listed

Read from `types.rs` (the only authored surface), so these are the options the
next pass can actually build against. Sizes are the same S/M/L the rest of the
file uses; **all of them are blocked on the migration itself**, which does not
compile yet.

| group field | what it buys our chat | work |
|---|---|---|
| `inference.allow_multiple_tool_calls` | parallel tool calls — a turn that reads three files stops being three round-trips | **SHIPPED 2026-11-21** as the Runtime tab's `parallel_tools` switch (default on; creation-time, so a flip answers `restart: true`). Verified by the unit test that the flag gates a restart and by the 58 CLI tests — not by a live turn, because the migration below only just compiled |
| `interaction.on_elicitation_request` + `interaction.ask_user_variant` | the structured-question row from Tier 2 #4 — **but it is L, not M.** `ElicitationRequest.requested_schema` is a JSON schema the host renders as a form and `ElicitationResult.content` is what gets submitted, so reusing `askcard` would `accept` a form with the wrong payload | L, needs a form renderer |
| `execution.limits` | **measured 2026-11-21 — not the runaway cap this row assumed.** `SessionLimitsConfig` has one field, `max_ai_credits`: a Copilot-plan credit budget. Nothing in 2.0.1 caps a turn except our own `timeout_secs`, which already exists | nothing to build |
| `agents.default` / `agents.selected` / `agents.included_builtin_agents` | a real agent picker, where 1.0.28 needed `custom_agents` built by hand | L |
| `state.memory_store` | a memory backend we choose, instead of "the runtime's store or nothing" | L |
| `policy.allowed_models` | refuse a model the endpoint cannot serve before the turn fails | S |
| `filesystem.additional_directories` | a second readable root without widening the path fence | S |
| `mcp.disable` | turn a server off at creation, instead of enabling then disabling it | S |
| `inference.enable_citations` | citations on answers, when the provider supports them | S |

The three that change behaviour rather than plumbing are the first three:
`allow_multiple_tool_calls` is wall-clock, elicitation is fidelity, and
`execution.limits` is the only thing in this list that stops a turn running away
with the budget. Build those first once the session creates.

### What stayed the same

`PermissionHandler`, `ToolHandler`, `ToolInvocation`, `ToolResult` and the
`types` / `rpc` / `session_events` / `handler` / `hooks` / `tool` / `session`
module paths are unchanged, and the feature names we pass (`runtime`,
`network-forwarding`, `bundled-cli`) still exist. So the chat's own code — the
seven tools, `Fence`, `WebPermission`, `WebInput`, every route — is not what this
migration rewrites. `connect()` is.
