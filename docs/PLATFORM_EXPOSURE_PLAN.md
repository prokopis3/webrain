# webrain — Platform Exposure Plan

> How to serve webrain as a **shared MCP tool** for free users and a
> **leased/paid** tool for entitled users, across three services:
>
> | Project | Stack | Path | Role |
> |---|---|---|---|
> | **webrain** | Rust | `Rust/webrain` | MCP scraping server (the engine) |
> | **deepscrape** | Angular | `Angular/deepscrape` | Frontend + BFF + Firebase functions |
> | **arachnefly** | Go | `Go/deepscrape/arachnefly` | Deploy, auth, paywall, quota, autoscale on Fly.io |
>
> `Go/deepscrape/` is a Go workspace (`go.work`) that also holds
> `deploycrawl4ai/` and `fly-log-shipper/`, with `fly-autoscaler*.toml` driving
> scale from queue depth. Every claim below was verified against source; the
> code citations name the file.

---

## 1. What already works in our favour

These are verified capabilities, not plans.

| Capability | Evidence |
|---|---|
| HTTP MCP transport | `webrain mcp --http <port>` → `webrain_mcp::run_http(&addr)` (`webrain-cli/src/main.rs`) |
| Bind address is configurable | `WEBRAIN_HTTP_ADDR`, default `127.0.0.1:<port>` |
| Per-connection session isolation | one CDP backend per connection, `Mcp-Session-Id` map, bounded so a long-lived server cannot leak sessions (`webrain-mcp/src/lib.rs`) |
| Bounded request body | `Max accepted HTTP request body` cap before parse |
| Flaky-accept tolerance | a reset connection logs and keeps serving instead of killing the server |
| Container-friendly | Docker entrypoint defaults to stdio; `--http` for a service |
| Zero-LLM extraction | extraction never calls a model, so per-request cost is CPU only |

The engine is already shaped like a service. That is why this plan is short.

---

## 2. The four blockers (verified gaps)

### 2a. The HTTP endpoint is unauthenticated — by design

`webrain-cli/src/main.rs` says it outright:

> `// Never bind the unauthenticated MCP endpoint to a LAN by default.`

`run_http` binds a bare `tokio::net::TcpListener` and serves every request.
There is no `Authorization` handling anywhere in `webrain-mcp`. The loopback
default is the only protection.

**Consequence:** webrain may never have ingress. Auth must live in front of it.

### 2b. Credential and profile state is process-global, not per-session

`webrain_profiles` lists the vault; `launch` and `login` operate on
`service:profile` read from it. The `Mcp-Session-Id` map isolates **CDP
backends**, but the vault and profiles are shared on-disk state for the whole
process.

**Consequence — this is the one that dictates the architecture.** On a shared
instance, tenant A's `webrain_session(op=login)` writes a profile that tenant B
can then use. Session-level isolation is therefore **not** a tenancy boundary.
The process/container is. Any "share one webrain among free users" design must
either disable stateful ops or accept cross-tenant leakage.

### 2c. No health route

A grep for `health | readyz | livez` across `webrain-mcp/src` returns nothing.
`webrain --doctor` exists, but as a CLI diagnostic, not an HTTP endpoint.

**Consequence:** `fly-autoscaler.toml` cannot health-check a new instance before
routing to it, and a rolling deploy cannot distinguish "warming" from "ready".

### 2d. No graceful shutdown

No `SIGTERM` handling in the HTTP server. Scale-in kills a process mid-scrape,
which loses in-flight batch work and leaves CDP tabs orphaned on the browser
side.

---

## 3. Target architecture

The unit of tenancy is the **container**, not the MCP session. Everything
follows from 2b.

```
                    ingress: HTTPS + Bearer
Angular / MCP client ───────────────────────► arachnefly (Go, Fly.io)
                                                  │  auth · paywall · quota
                                                  │  tenant → worker routing
                                                  ▼
                                   Fly private 6PN only, no public ingress
                              ┌───────────────┴───────────────┐
                              ▼                               ▼
                    free pool                           leased instance
              N anonymous sessions                 one tenant, full state
              stateful ops DISABLED                  vault + profiles enabled
```

- **Free tier** — shared container(s), many anonymous sessions, `vault` /
  `launch` / `login` / `session(login)` **refused**, aggressive per-session
  concurrency caps, short request timeout.
- **Paid tier** — a leased container per tenant (or per org), stateful ops
  enabled, sticky routing so a session lands on the same instance.
- **Both** — arachnefly is the only thing with ingress. Workers are reachable
  over 6PN only.

Diagrams: `docs/diagrams/platform-services.mmd`,
`docs/diagrams/webrain-mcp-exposure.mmd`,
`docs/diagrams/scrape-request-flow.mmd`.

---

## 4. Changes webrain needed (5, all small) — SHIPPED 2026-09-30

Ordered by whether they block the free tier. All five are implemented; the notes
below record what shipped and, in 4c, where a deliberate simplification was made
instead of building what this plan originally proposed.

### 4a. `no_state` mode — blocks the free tier (P0)

Refuse the stateful tools when an env flag is set, so a shared container cannot
leak profiles between tenants. One guard where the tools are dispatched, not one
per tool.

```
WEBRAIN_NO_STATE=1  →  vault*, launch, login, session(op=login) return a typed
                       error: "stateful ops disabled on a shared instance"
```

**Why one guard:** every stateful tool routes through the same dispatcher, so a
single check covers all of them and cannot drift. A per-tool guard would leave
siblings open — the exact failure mode a shared pool must not have.

**~25 lines**, `webrain-mcp/src/lib.rs` + `tools.rs`.

### 4b. Optional bearer auth — defence in depth (P0)

```
WEBRAIN_HTTP_TOKEN=<secret>  →  require  Authorization: Bearer <secret>
```

Unset = current behaviour, so local and stdio users are untouched. It is not a
substitute for arachnefly; it is what makes a mis-published port survivable.

**~20 lines**, `webrain-mcp/src/lib.rs::run_http`.

### 4c. `/health` (P1) — shipped as ONE route, not two

Unauthenticated, cheap, no CDP touch. Required for autoscaler admission and
rolling deploys. Unauthenticated by design, so the probe needs no credential.

`/readyz` was **deliberately not built.** CDP backends attach per session, so a
process that is listening is genuinely ready to serve: a readiness endpoint that
always returns 200 next to a liveness endpoint that always returns 200 is a
distinction with no information behind it, and two routes to keep honest. Add
`/readyz` only when a real warm-up step exists (for example a pre-attached
engine), at which point it starts telling the truth.

**~25 lines**, same file.

### 4d. Graceful shutdown (P1)

On SIGTERM: stop accepting, let in-flight tool calls finish under a deadline,
close tabs, exit. Prevents losing batch work on scale-in.

**~30 lines**, `webrain-cli/src/main.rs` + `run_http`.

### 4e. Per-session concurrency cap (P1)

The session map is bounded, but a single session can still request a large
`concurrency` in `webrain_batch`. On a shared instance one tenant can starve the
others. Cap it per session when `no_state` is on.

**~15 lines**, `tools.rs` batch handler.

Total ≈ 115 lines, no new dependencies. That is the whole engine-side cost of
becoming a multi-tenant service.

---

## 5. Phased plan

| Phase | Deliverable | Depends on |
|---|---|---|
| **P0** | `no_state` + bearer auth in webrain; container image with `WEBRAIN_HTTP_ADDR=0.0.0.0:9223`, 6PN only, no public ingress | — |
| **P0** | arachnefly: entitlement gate + tenant→worker routing + per-tier quota; free/paid split | webrain P0 |
| **P1** | `/health` + `/readyz` + graceful shutdown wired into `fly.toml` health checks and autoscaler admission | webrain P0 |
| **P1** | deepscrape BFF: replace direct scrape calls with MCP calls through arachnefly; surface `challenge` and quota errors in the UI | arachnefly P0 |
| **P2** | Leased instances with sticky routing for paid tenants; per-tenant proxy lanes (`WEBRAIN_PROXIES` per tenant) | all of the above |
| **P2** | Observability: request/token accounting per tenant from the existing `fly-log-shipper` | P1 |

Browser choice per tier matters: the free pool should run **obscura or
lightpanda** (CDP over a container), not real Chrome. Real Chrome is the paid
path — it is the only one that passes interactive challenges, and it is too
heavy to share.

---

## 6. What NOT to do

- **Do not put webrain behind public ingress**, even with 4b. One secret in
  front of a shared, stateful process is not a tenancy boundary.
- **Do not treat the MCP session as a tenant.** 2b is `ponytail:`-level explicit
  in the code: profiles are process-global. Session-level isolation would be a
  confident wrong fix.
- **Do not add billing, metrics, or rate limiting inside webrain.** `serp.rs`
  already defers exactly these as SaaS concerns; arachnefly is the right owner
  and already has the deploy surface.
- **Do not build a second HTTP API in webrain for the frontend.** MCP is the
  contract; arachnefly adapts it. A parallel REST surface would be a second
  thing to keep in sync.

---

## 7. Verified facts this plan rests on

- `webrain mcp --http <port>` → `run_http`; `WEBRAIN_HTTP_ADDR` overrides the
  bind address; loopback default with an explicit "never bind unauthenticated to
  a LAN" comment — `webrain-cli/src/main.rs`.
- `run_http` binds `TcpListener`, has a bounded `Mcp-Session-Id` map, a max body
  cap, and tolerates accept errors — `webrain-mcp/src/lib.rs`.
- **As of 2026-09-30 the transport gaps 2a–2d are closed and empirically tested.**
  `scripts/probe_http_transport.ps1` starts the server and probes every guard. With
  `WEBRAIN_NO_STATE=1` and `WEBRAIN_HTTP_TOKEN` set: `GET /health` → 200
  unauthenticated; `tools/list` with no header → 401; with a wrong token → 401;
  with the right token → 200; `webrain_profiles` → refused; `webrain_session(op=login)`
  → refused by the *same* guard via `map_surface`; `webrain_eval` → passes the guard
  and reaches the CDP layer, proving the guard stays narrow. With the flags
  **unset**: auth is off (all four auth probes 200) and `webrain_profiles` returns
  the real profile list — strictly additive, no regression when disabled.
- **Correction (2026-10-01): the guard was not sufficient, and "guards the stateful
  tools" was the wrong mental model.** `webrain_session` folds into eight executors and
  the original list named only two of them, so `op=list` (enumerating every tenant's
  sessions) and `op=cookies`/`op=save_state` (handing over cookies and auth state) were
  served on a shared worker. Three of those — `open`/`close`/`list` — are answered by the
  **outer HTTP layer** and never reach `handle_rpc`, so the guard's placement there could
  not have covered them even in principle. Two lessons worth keeping: a guard keyed on
  *executor* names is defeated by any consolidation layer that fans one tool into many,
  and a guard in one of two dispatch layers is a guard on one of two doors. The guard now
  matches the umbrella in both layers, and §2b's premise ("the container is the tenancy
  unit") remains the honest position — this closes a hole, it does not make a shared
  worker a tenancy boundary.
- That last result is also the evidence for **blocker 1**: `webrain_profiles`
  returned live usernames out of the shared store, so on a shared instance any
  tenant can read — and then log into — another tenant's accounts.
- `WEBRAIN_HTTP_TOKEN` is compared in constant time and enforced only when set; an
  unset token leaves localhost behaviour byte-for-byte unchanged.
- **Not tested: the SIGTERM drain.** It is unix-only (`tokio::signal::unix`), and
  this was verified on Windows, where stopping a process is `TerminateProcess` —
  there is no signal to catch. The drain loop compiles and shares its path with
  `ctrl_c`, but treat "graceful scale-in" as unverified until it runs on Linux.
- Vault/profiles are process-global: `webrain_profiles` lists them, `launch` and
  `login` take `service`/`profile` from that shared store — `webrain-mcp/src/lib.rs`.
- arachnefly is a Go service with `domain/ infrastructure/ interfaces/ views/`,
  `Dockerfile`, `fly.toml`, `fly-autoscaler.toml`, inside a Go workspace that
  also builds `deploycrawl4ai` and `fly-log-shipper` — `Go/deepscrape/`.
- deepscrape is Angular with `bff/`, `functions/`, `deploy/`, and Firebase
  config — `Angular/deepscrape/`.

**Not verified** (needs a look before P2): arachnefly's actual route surface and
whether its domain layer already models tenancy/entitlement, and the deepscrape
BFF's current scrape path. Both live outside this repository and were read only
at directory level.
