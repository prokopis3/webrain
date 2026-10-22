# ADR-002: Intent-Based MCP Tool Surface with Centralised Dispatch

- **Status:** Accepted (2026-09-30)
- **Project:** webrain (`d:\Windows\Documents\Programming\Projects\Rust\webrain`)
- **Related docs:** [ADR-001](./0001-webrain-architecture.md), `docs/ARCHITECTURE.md`,
  `docs/reference/tools.mdx`, `webrain-mcp/src/tools.rs`, `webrain-mcp/src/lib.rs`
- **Graph revision:** 2421 nodes / 5591 edges — indexed 2026-09-30 under
  codebase-memory-mcp 0.11.0, after the proxy/httpcache modules landed

## Context

[ADR-001](./0001-webrain-architecture.md) fixed the *layering* decision: a three-crate
Cargo workspace with a CDP backend, an MCP transport, and a thin CLI. It did not record the
*shape of the agent-facing contract*.

That shape is a real decision, because two forces pull against each other:

1. **LLM ergonomics.** The agent's prompt is the only source of intent. A tool surface must
   be small, stable, and generic enough to serve any domain without per-site recipes.
2. **Engine reality.** The engine underneath is large. `CdpBackend` alone exposes
   **66 methods** (`webrain-core/src/backends/cdp.rs`).

Mirroring the engine would mean publishing 66 tools. That surface is too large to document
accurately (a previous 16-vs-17 drift in the docs shows how easily the count desynchronises)
and too large for an agent to select from reliably.

## Decision

Publish a **17-tool intent surface** and absorb the mismatch in a translation layer.

1. **17 intent-based MCP tools** are the public contract:
   `guide`, `eval`, `eval_in_frame`, `navigate`, `observe`, `interact`, `extract`, `scrape`,
   `batch`, `crawl`, `search`, `serp`, `pdf`, `download`, `watch`, `session`, `vision`.
2. **Dispatch centrally** in `call_tool()` (`webrain-mcp/src/tools.rs`), with
   `handle_rpc()` (`webrain-mcp/src/lib.rs`) driving both stdio and HTTP transports, and the
   CLI mirroring the same shape with a `match` in `main()` — deliberately **no clap**.
3. **Fold consolidated calls back** onto legacy executor arms via `map_surface()`, so
   consolidated and legacy callers share one executor and no logic is duplicated.
4. **Keep schemas declarative.** `list_tools()` is a literal schema block, not generated.

## Consequences

### Positive

- One surface stable enough to document once and drive from four or more agent hosts.
- A single obvious place to add a tool: one match arm plus one schema literal.
- **Zero framework dependencies** — no clap, no schema-derive crate — so the binary stays
  small and the consolidation stays visible instead of hidden behind a macro.
- CLI and MCP share one engine (`webrain-core` ships no binary), so no logic is duplicated.

### Negative (measured 2026-09-30 from the knowledge graph)

Dispatch is concentrated into very few, very large functions:

| function | file | complexity | lines |
|---|---|---|---|
| `call_tool` | `webrain-mcp/src/tools.rs` | **323** | **1315** |
| `handle_rpc` | `webrain-mcp/src/lib.rs` | 116 | 794 |
| `main` | `webrain-cli/src/main.rs` | 103 | 906 |
| `map_surface` | `webrain-mcp/src/tools.rs` | 83 | 160 |
| `list_tools` | `webrain-mcp/src/tools.rs` | 0 | 259 |

- `call_tool` alone is **54% of `tools.rs`** and the most complex function in the repo.
- Roughly **3,200 lines of dispatch** live in those four functions.
- The MCP layer is **coupled to CDP specifics**: `call_tool` has 75 distinct
  statically-resolved callees, **50 of them `CdpBackend` methods**. The "transport" crate
  therefore knows engine internals.
- **God-object pressure** on `CdpBackend` (66 methods).
- Leiden analysis puts the vault + download/whisper group at cohesion **0.26** — the loosest
  of the 12 communities — so those concerns are grouped by convenience rather than by domain.

### Neutral

`list_tools()` is 259 lines but has complexity 0: it is data, and it diffs cleanly.

## Alternatives considered

- **Mirror `CdpBackend` 1:1 (66 tools).** Rejected: unstable for LLMs, impossible to
  document accurately, and it makes the agent choose among engine internals rather than
  intents.
- **clap + derive-based CLI/MCP.** Rejected: a dependency cost for behaviour a `match`
  already provides, and it obscures the consolidation.
- **Split `call_tool` into per-tool handlers now.** Deferred: churn without a forcing
  function. The measured cost is real but bounded, and `map_surface()` already provides the
  seam to split along.

## Revisit when

- A fourth crate or a second transport lands, or `call_tool` gains another tool family.
- `CdpBackend` grows materially past 66 methods.
- The vault + download group's cohesion keeps falling.

## Network plane (2026-09-30)

Two cross-cutting modules landed in `webrain-core`, and each encodes a decision rather
than an implementation detail:

- **`proxy`** — rotation is opt-in via `WEBRAIN_PROXIES`; an explicit per-call `proxy`
  always wins and never rotates, so single-proxy deployments behave exactly as before.
  Rotation fires only on evidence: a 403/429/503 or a transport failure. A 2xx never
  rotates, even when it parses to zero results — the exit answered, so a different IP
  would not help. A walled exit cools down for 120s rather than being banned outright,
  and when *every* exit is cooling down one is still returned: a pool that answers
  "no proxy" would strand a crawl mid-run.
- **`httpcache`** — revalidation-only. A 304 is taken as the origin vouching for the
  stored body, so nothing stale-without-blessing is ever served; a 200 replaces the entry.
  Keyed by `(proxy, url)`, because the same URL through a different exit is not the same
  resource. Two layers: memory holds validators **and bodies** (256 entries, 900s TTL);
  a disk layer holds **metadata only** — validator pair, body hash, check time, keyed by a
  hash of `(proxy, url)` — so a restarted process still sends `If-None-Match` and scheduled
  recurring monitoring does not pay for a full download each run. Bodies are never written
  to disk. Disk is bounded at 1024 entries / 7 days and lives at `WEBRAIN_HTTP_CACHE_DIR`
  or the platform cache dir. Kill switch `WEBRAIN_HTTP_CACHE=0` covers both layers.

The scope boundary is the part worth remembering: **browser-side rotation is a pool of
browser processes, not a per-request switch.** CDP has no per-context proxy, so a
`CdpBackend`'s proxy is fixed at launch and the honest unit of isolation is the engine —
which is precisely what `webrain_batch`'s `cdp_urls` already provides. Any future "rotate
the proxy" request that targets the browser path should be answered with a browser pool,
not with a per-request hook that cannot exist.

## Graph evidence

Method: `get_architecture` (clusters, hotspots, layers), `query_graph` (complexity, line
spans, per-file function counts, coupling), `trace_path` (`call_tool` outbound calls).

- Nodes/edges: 2366 / 5368. Labels: 342 functions, 158 methods, 32 structs, 1 trait.
- Fan-in hotspots: `TileEngine.new` (103), `VectorStore.is_empty` (84),
  `video.Detail.as_str` (83), `vault.get` (68), `VectorStore.len` (58),
  `CdpBackend.ensure_page_attached` (29), `vault.now` (28), `CdpBackend.send_cmd` (27),
  `CdpBackend.eval_js` (25).
- Function counts per file: `engines.rs` 47, `serp.rs` 46, `video.rs` 40, `install.rs` 34,
  `vault.rs` 28, `tools.rs` 19, `main.rs` 16, `cdp.rs` 15.
- 12 Leiden clusters, cohesion 0.26–1.00: CDP core (66 members, 0.57), dispatcher spanning
  webrain-mcp + core + cli (56, 0.39), install/engine-discovery (48, 0.68), vault (15, 0.62),
  docs (13, 1.00).
- Entry points: `webrain-cli/src/main.rs::main`,
  `.github/scripts/check_docs_orphans.py::main`.

## Tooling note (2026-09-30)

`codebase-memory-mcp` 0.11.0 refuses this repository's path as a *system* directory, because
its first component on `D:` is literally `Windows` (`WS_WINDOWS_SYSTEM_TREES` is matched on
that component alone). The refusal is overridable, but `allow-root --approve-sensitive`
stores the path with backslashes while the indexer canonicalizes to forward slashes, so the
exact-match lift in `cbm_workspace_root_allowed` never fires.

Working approval — append the forward-slash form to
`~/.cache/codebase-memory-mcp/allowed_roots`:

```
!D:/Windows/Documents/Programming/Projects/Rust/webrain
```
