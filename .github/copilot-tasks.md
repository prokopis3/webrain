
# Tasks — webrain

Current backlog, ordered. Each item names its file and approximate size, and every
item traces to a verified gap — nothing here is speculative.

> **P0 and P1 shipped 2026-09-30.** `WEBRAIN_NO_STATE`, `WEBRAIN_HTTP_TOKEN`
bearer auth, `GET /health`, the SIGTERM drain, and the shared-instance concurrency
> cap all landed in `webrain-mcp`. P2 is still open.

## P0 — blocks the shared / free tier

- [ ] **`WEBRAIN_NO_STATE` guard.** Refuse `vault*`, `launch`, `login`,
      `session(op=login)` when set. **One check in the dispatcher**, not one per
      tool: every stateful tool routes through the same place, so a per-tool
      guard would leave siblings open — the exact failure a shared pool must not
      have. (~25 ln, `webrain-mcp/src/lib.rs`)
- [ ] **Optional bearer auth.** `WEBRAIN_HTTP_TOKEN` → require
      `Authorization: Bearer <secret>`. Unset = today's behaviour, so local and
      stdio users are untouched. Not a substitute for arachnefly; it is what
      makes a mis-published port survivable. (~20 ln, `run_http`)

## P1 — required for Fly autoscale and rolling deploy

- [ ] **`/health` + `/readyz`.** Unauthenticated, no CDP touch; `/readyz` returns
      503 until the backend is attached. `fly-autoscaler.toml` currently has
      nothing to check. (~25 ln)
- [ ] **Graceful shutdown on SIGTERM.** Stop accepting, drain in-flight tool
      calls under a deadline, close tabs. Scale-in currently loses batch work.
      (~30 ln)
- [ ] **Per-session concurrency cap** when `no_state` is on — one tenant can
      otherwise starve the others with a large `webrain_batch` `concurrency`.
      (~15 ln, `tools.rs`)

## P2 — platform

- [ ] Proxy lanes per tenant (`WEBRAIN_PROXIES` is process-global today).
- [ ] Leased-instance sticky routing (arachnefly side).
- [ ] deepscrape BFF: route scrapes through arachnefly; surface `challenge` and
      quota errors in the UI.

## Docs out of sync

- [ ] `docs/INTEGRATION_PLAN.md` is stale — it claims "zero distribution" while
      the repo ships Scoop / Homebrew / Docker artifacts and install scripts, and
      it lists `webrain_fetch_http` as CDP-dependent when a no-browser path
      exists.
- [ ] `docs/FEATURE_GAPS.md` build order still shows 9b (proxy rotation pool)
      and the 304 cache as unbuilt; both shipped 2026-09-30.

See `docs/PLATFORM_EXPOSURE_PLAN.md` for the reasoning behind P0/P1.

