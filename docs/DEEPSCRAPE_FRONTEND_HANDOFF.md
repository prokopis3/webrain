# deepscrape frontend — audit and webrain integration handoff

Audit date 2026-09-30. deepscrape `0.9.0-beta.2`, branch `next`
(`D:/Windows/Documents/Programming/Projects/Angular/deepscrape`, 17,967 nodes /
32,004 edges in the codebase-memory graph).

Scope note: this document **analyses** deepscrape and **does not modify it**. It lives
in webrain because the platform plan is authored here
(`docs/PLATFORM_EXPOSURE_PLAN.md`). The actionable prompt for the next agent is
`docs/prompts/deepscrape-frontend-improve.prompt.md`.

---

## 0. The question that prompted this: is webrain integrated?

> **SUPERSEDED 2026-10-01 — read this box before the rest of the document.**
> The audit below was accurate when written (2026-09-30) and is now **out of date**: the
> integration landed within a day. deepscrape now ships
> `functions/src/infrastructure/webrain-mcp.ts` (an MCP JSON-RPC client that unwraps the
> SSE/result envelope), `webrain-config.store.ts` (Secret Manager-backed token + proxy
> pool, 30s TTL, runtime-rotatable), `domain/webrain-tools.domain.ts` (a **fail-closed**
> tier allowlist), `domain/webrain-proxy.domain.ts` (platform-owned egress), plus
> `WebrainConfigService`, `WebrainToolsService`, a `tools-panel` component and an admin
> panel on the frontend, and a smoke test (`functions/scripts/webrain-smoke.ts`,
> `functions/scripts/probe-webrain-mcp.ps1`).
>
> Two consequences for the text below. Finding **S1 is fixed** — the caller's token relay
> was removed in `ai_handler.ts:85` and replaced with the server-held secret, with the
> verification note that relaying a user token got a 401 anyway. Findings on the
> *crawl4ai* path (S2, S3, P1, P2) and on the frontend (F1-F4) remain accurate.
>
> And one finding **in the other direction**, which is the more useful part: the
> integration had to rebuild the shared-instance boundary in front of webrain, because
> webrain's `WEBRAIN_NO_STATE` did not hold — see §3's note and webrain's CHANGELOG. That
> is fixed now, but it is the cautionary example that belongs in any future plan: a guard
> keyed on *executor* names is defeated by a consolidation layer that fans one tool into
> many, and a guard in one of two dispatch layers only closes one door.
>
> Everything below is retained because its reasoning still explains **why** the adapter
> belongs in deepscrape rather than in webrain.

**Not integrated as of 2026-09-30 (superseded by the box above).** A repo-wide search for
`webrain` in deepscrape returned 0 matches at audit time. webrain was not referenced in
the frontend, the BFF, the functions, or the env files.

What deepscrape talks to instead:

| Path | Goes to | Evidence |
|---|---|---|
| `CrawlAPIService.sendToCrawl4AI` | **crawl4ai, directly from the browser** | `src/app/core/services/crawlapi.service.ts:20-28` |
| BFF `POST /api/crawl` | crawl4ai via `API_CRAWL4AI_URL` | `functions/src/handlers/ai_handler.ts:60` |
| `CrawlerHandler` (job pipeline) | crawl4ai job/stream endpoints | `functions/src/handlers/crawler.ts` |
| Machine lifecycle | **arachnefly** | `src/app/core/variables/deployment.var.ts:3-4`, `deployment.service.ts` |

So the platform is already a two-service system (deepscrape ↔ crawl4ai ↔ arachnefly) with
webrain absent. Everything webrain gained recently — the `WEBRAIN_NO_STATE` /
`WEBRAIN_HTTP_TOKEN` guards, proxy rotation, the revalidation cache — is *enablement*
for a future integration, not an integration.

---

## 1. What exists in the frontend today (so improvements build on it)

Angular 20.3 (`@angular/build` 20.3.36), **zone.js still enabled**, Tailwind 3.4
alongside **108 SCSS files**, Firebase (Firestore + functions), Stripe billing,
org-level multi-tenancy, Bun-run BFF on Cloud Run, semantic-release + commitlint + husky.

The crawl feature is already built out, which is why the integration is additive rather
than new work:

- `src/app/pages/user/crawl-pack/` — the crawl page, incl. `crawl-config/`
- `src/app/core/components/app-crawl/`, `crawl-result-item/`
- `src/app/core/services/crawlapi.service.ts` — `CrawlAPIService`
- `src/app/core/services/crawl-stream.service.ts` — `CrawlStreamService`
- `src/app/core/types/crawl-result.type.ts` — `CrawlResult`
- Domain types already present: `CrawlOperation, CrawlPack, CrawlStatus, CrawlStreamBatch,
  CrawlTask, JinaOptions`
- Firestore-backed paging functions already exist for `BrowserProfiles`, `CrawlConfigs`
  and `CrawlResultConfigs` (`getBrowserProfilesPaging`, `getCrawlConfigsPaging`,
  `getCrawlResultConfigsPaging` in `functions/src/index.ts`)

That last row matters: **deepscrape already models browser profiles and crawl configs in
its domain.** webrain's `vault` + `profiles` (and TOTP) map onto concepts that already
exist rather than introducing new ones.

deepscrape also has its own ADRs worth reading before changing authz:
`docs/ADR-2026-03-AUTHZ-REBAC-ABAC.md`, `docs/ADR-2026-09-DOMAINS-MULTISITE-SPLIT.md`.

---

## 2. Findings

Ordered by severity. Every one is cited to a line range that can be re-checked.

### Security

**S1 — the user's bearer token is handed to crawl4ai. (High)**
`crawlapi.service.ts:47-50` sets `'api-key': \`Bearer ${this.authService.token}\`` and
`'Authorization': \`Bearer ${this.authService.token}\`` on a request the *browser* makes
to `environment.API_CRAWL4AI_URL`. The caller's session token is disclosed to a third
party and, if crawl4ai is not ours, that is credential leakage with no compensating
benefit. The crawl4ai API key should be a BFF-held secret; the browser should never
present the user's token to it.

**S2 — the SSRF guard covers only one of two paths. (High)**
`ai_handler.ts:66-84` is a careful guard: scheme allowlist, RFC1918/link-local/metadata
`BLOCKED_HOST` regex, 2048-char cap, `.slice(0, 50)`. But the *browser* path in S1 posts
`{urls: encodedUrl}` to crawl4ai without passing through the BFF at all. So the guard
protects `/api/crawl` while the primary client path is unguarded. A guard on one of two
doors is not a guard.

**S3 — the guard is pre-redirect only, and the code says so. (High)**
`ai_handler.ts:61-65` contains a candid comment: an unvalidated list here is an
internal-read primitive, and *"the agent needs the same list re-checked after redirects,
which is the half this repo cannot do for it."* A permitted host that 302s to
`169.254.169.254` defeats the filter. This must be enforced where the request is
actually made (crawl4ai/arachnefly: no-follow plus re-validation, or a network-level
egress policy). Worth noting the same file already fixed a sibling issue — the
`x-set-cookie` passthrough was removed for session fixation (`ai_handler.ts:103-105`),
so there is precedent for closing these properly.

**S4 — the API key is never applied. (Medium, likely a bug)**
`crawlapi.service.ts:29`:
```ts
const encodedUrl = environment.CRAWL4AI_API_KEY !== '' ? url : url
```
Both branches return `url`. The ternary is a no-op, so `CRAWL4AI_API_KEY` has no effect
and `customUrlEncoder` is bypassed (it is commented out on the same line). Either the
key is meant to be used or the variable is dead; right now it reads as a silent
misconfiguration.

### Correctness and performance

**P1 — the crawl response is fully buffered, and streaming was abandoned. (Medium)**
`ai_handler.ts:117` does `await apiResponse.arrayBuffer()` and then
`res.end(Buffer.from(buffer))`. A whole crawl result is materialised in memory in a
Cloud Run container before being handed on. Someone clearly attempted the right thing —
`handleStreamedApiResponse` with `streamPipeline(apiResponse.body, res)` sits commented
out at `ai_handler.ts:25-57`, `streamPipeline`/`promisify`/`pipeline` imports are
commented at lines 5-6 and 10, and the call site is commented at line 124. Restoring that
pipeline is the single highest-value performance fix in the crawl path.

**P2 — JSON is carried as `application/octet-stream`. (Medium)**
The handler sets `Content-Type: application/octet-stream`
(`ai_handler.ts:119`), so the client must `JSON.parse(arrayBufferToString(response))`
(`crawlapi.service.ts:60`) and declares `responseType: 'arraybuffer'`. There is no typed
contract, no content-type, and a parse failure surfaces as a generic conversion error
rather than "the crawler returned a challenge page".

**P3 — endpoints are derived by string replacement. (Low)**
`crawlapi.service.ts:78` builds the job endpoint with
`this.crawl4AiEndpoint.replace('/crawl', '/job')`. Any future path containing `/crawl`
breaks it, and the intent is invisible. Three separate URL constants would be clearer
and cheaper.

**P4 — dead code in the hot path. (Low)**
`ai_handler.ts` carries ~45 lines of commented-out implementation plus three commented
imports; `crawlapi.service.ts` has commented `X-With-*` header blocks and a commented
`console.log`. Dead code in the file you debug most is a real cost.

### Platform and frontend

**F1 — zone.js is still enabled on Angular 20. (Medium perf)**
`zone.js ~0.15.1` is a runtime dependency, so change detection is still zone-driven.
Angular 20 supports `provideZonelessChangeDetection()`, and this app is a
Firestore-heavy dashboard, which is exactly the shape that benefits.

**F2 — Tailwind and SCSS both style the app. (Medium)**
Tailwind 3.4 (`tailwindcss ^3.4.19`) plus 108 SCSS files. Two systems means two sources
of truth for spacing, colour and breakpoints. Tailwind 4 is also available and is a
CSS-first rewrite — a separate, larger decision.

**F3 — env parsing is triplicated. (Low)**
`src/config/env.ts` (a `getEnv` at 51-108), `functions/src/config/env.ts` (a larger
`getEnv` at 156-253), and a further `config/` copy. Three parsers for one concept drift
independently.

**F4 — there is no `challenge` concept anywhere. (High, product)**
Grepping deepscrape for challenge/anti-bot vocabulary returns nothing in the crawl path.
The UI therefore cannot distinguish *"this site blocked us"* from *"this site was empty"*.
webrain returns a `challenge` field on every `navigate`/`batch` (`cloudflare_challenge` /
`blocked` / `captcha`) precisely so the caller can tell those apart and escalate to real
Chrome plus a session. This is the clearest product win available and it is a UI
improvement, not just plumbing.

---

## 3. What webrain offers against those findings

| deepscrape need | webrain surface | Note |
|---|---|---|
| Typed results, not a blob (P2) | `webrain_extract_json`, `autoschema`, `table`, `bm25` | zero-LLM; no raw HTML crosses the boundary |
| "Were we blocked?" (F4) | `challenge` on `navigate`/`batch` | drives a distinct UI state and an escalation CTA |
| Profile reuse (S1, F4) | `vault` + profiles + TOTP | binds to the existing `getBrowserProfilesPaging` |
| Parallelism | `webrain_batch` (`concurrency`, `cdp_urls`) | one CDP WS, independent tabs |
| Redirect-safe targets (S3) | `validate_urls` | plus never letting the client pick the target |
| Shared-instance safety (S1) | `WEBRAIN_NO_STATE`, `WEBRAIN_HTTP_TOKEN` | shipped and probe-tested 2026-09-30 |

> **Cautionary note (2026-10-01).** deepscrape found that `WEBRAIN_NO_STATE` was **not**
> sufficient as first shipped: it matched three executor names while `webrain_session`
> folds into eight, and three of the eight were answered by the outer HTTP layer and never
> reached the guard. `op=list` enumerated other tenants' sessions. It is fixed now (the
> guard matches the umbrella in both layers), but the integration still had to add its own
> fail-closed allowlist in front of the worker, and that is the right call — webrain's own
> guard is defence in depth, **not** a substitute for a gateway that knows who is calling.
| Steadier egress | `WEBRAIN_PROXIES` rotation | process-global — see the warning below |

Two constraints that must survive the integration:

1. **Do not route tenants through a shared `WEBRAIN_PROXIES` pool.** It is process-global
   and rotates on 403/429, so one tenant hitting a wall would rotate another tenant's
   traffic. Per-tenant proxy lanes are a planned P2, not a current feature.
2. **Do not push deepscrape-specific behaviour into webrain.** No per-site recipes, no
   hardcoded selectors, no intent sniffing (`AGENTS.md`, and `docs/PLATFORM_EXPOSURE_PLAN.md`
   §6). If deepscrape needs it, it goes in deepscrape's adapter.

---

## 4. How the frontend should change

The shape that respects the existing code rather than replacing it:

```
Angular CrawlAPIService            (keep the class, change the transport)
  -> BFF /api/crawl                (keep the route; it is where authz and CSRF already live)
     -> arachnefly                 (already the machine/entitlement owner)
        -> webrain worker          (private 6PN, WEBRAIN_NO_STATE=1, bearer token)
           -> typed JSON + challenge
```

Frontend-visible changes, smallest first:

1. **Stop sending the user token to crawl4ai** (S1). The BFF holds the crawl credential.
   This is a security fix, not a refactor.
2. **Give `CrawlResult` a `challenge` field** (F4) and add a UI state for it — the site is
   gated, offer "solve in a real browser / attach a session" instead of showing an empty
   result. This is the change users will actually notice.
3. **Restore streaming on the BFF** (P1) and set a real content-type (P2) so the client
   stops `JSON.parse`-ing a binary blob.
4. **Move single-URL crawl behind the same guarded route** (S2) so there is one door, not two.
5. Then the platform work: route through arachnefly to a webrain worker, and bind browser
   profiles to the existing `BrowserProfiles` collection.

Items 1-3 are independently shippable and do not require webrain at all. That ordering is
deliberate: the frontend should not wait on the platform integration to get safer and faster.

---

## 5. Not verified here

Read at directory and symbol level, plus targeted source reads — not run:

- Whether `environment.API_CRAWL4AI_URL` is publicly reachable from a browser in
  production. S1/S2 severity depends on this: if it is public, the browser bypass is live;
  if it is private, `CrawlAPIService` is broken in prod instead. **Check this first** —
  it changes the priority order.
- Whether arachnefly re-validates redirects (S3). Only its file layout was inspected here.
- The arachnefly route surface and whether its domain layer already models entitlement.
  `arachnefly` is indexed separately (784 nodes / 1,521 edges, branch `next`) and is the
  right next place to look.
- No deepscrape test suite was executed.
