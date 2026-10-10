---
title: deepscrape frontend — improve and wire to the crawl platform
target: Angular/deepscrape (frontend + BFF); read-only on webrain and arachnefly
prerequisite_reading: docs/DEEPSCRAPE_FRONTEND_HANDOFF.md, docs/PLATFORM_EXPOSURE_PLAN.md
---

# Prompt

You are a senior full-stack engineer working on **deepscrape**, an Angular 20 +
Firebase product that crawls and scrapes the web for paying customers. Your job is to
make its crawl path **safer, faster, and honest about failure**, and then to wire it to
the platform's scraping workers. Work in `Angular/deepscrape` only unless a task
explicitly says otherwise.

Do not start editing until you have completed Phase 0. Do not claim anything works
without running it.

---

## Context you must load first

Three repositories take part. Know which one you are in.

| Repo | Path | Branch | Stack | Your access |
|---|---|---|---|---|
| **deepscrape** | `D:/Windows/Documents/Programming/Projects/Angular/deepscrape` | `next` | Angular 20.3, Tailwind 3.4 + SCSS, Firebase functions, Bun BFF on Cloud Run, Stripe, Firestore | **read/write** |
| **arachnefly** | `D:/Windows/Documents/Programming/Projects/Go/deepscrape/arachnefly` | `next` | Go, DDD (`domain/ infrastructure/ interfaces/ views/`), Fly.io + `fly-autoscaler.toml` | read-only unless told otherwise |
| **webrain** | `D:/Windows/Documents/Programming/Projects/Rust/webrain` | — | Rust, MCP scraping server, 17 intent tools | **read-only** |

deepscrape is `0.9.0-beta.2`. It already has: org-level multi-tenancy, a REBAC/ABAC
authz model (`docs/ADR-2026-03-AUTHZ-REBAC-ABAC.md`), a multi-site domains split
(`docs/ADR-2026-09-DOMAINS-MULTISITE-SPLIT.md`), Stripe entitlements, CSRF on `/api`,
and — importantly — **browser profiles and crawl configs already modelled in Firestore**
(`getBrowserProfilesPaging`, `getCrawlConfigsPaging`, `getCrawlResultConfigsPaging` in
`functions/src/index.ts`). Bind to those; do not invent parallel concepts.

Current crawl flow, for orientation:

```
Angular CrawlAPIService ──► crawl4ai (directly, from the browser)      src/app/core/services/crawlapi.service.ts
Angular ──► BFF POST /api/crawl ──► crawl4aiCore ──► API_CRAWL4AI_URL  functions/src/handlers/ai_handler.ts
Angular ──► CrawlerHandler (enqueue/status/stream/cancel) ──► crawl4ai functions/src/handlers/crawler.ts
Angular ──► DeploymentService ──► arachnefly /machine/...              src/app/core/services/deployment.service.ts
```

deepscrape does **not** currently reference webrain anywhere. That is the gap the later
phases close.

### Tooling rules (non-negotiable)

- Prefix shell commands with **`rtk`**.
- Before touching deepscrape code, investigate it with the codebase-memory MCP graph.
  Project key: **`deepscrape`**. Cascade from cheapest to most expensive:
  `search_graph` → `trace_path` → `search_code` → `get_code_snippet` → `read_file` (last resort).
  `arachnefly` is a separate project key. Check `docs/` for the file you need before
  grepping the whole tree.
- Run `mcp_codebase-memo_detect_changes(project="deepscrape")` before and
  `mcp_codebase-memo_index_repository(repo_path="D:/Windows/Documents/Programming/Projects/Angular/deepscrape", mode="fast")`
  after your changes.
- deepscrape deploys with `npm run build`, `npm run deploy:functions`, `cd bff && bun run deploy:cloud`.
  **Never run a deploy.** Build and test locally only.

---

## Phase 0 — Verify the premise (do this before writing any code)

The audit that produced this prompt found the items below by reading code, not by running
it. **Confirm each one with your own evidence before acting on it**, and report any you
cannot reproduce instead of implementing a fix for it.

Investigate and record answers to these five questions:

1. **Is `environment.API_CRAWL4AI_URL` reachable from a browser in production?**
   This single answer reorders everything. If it is publicly reachable, the browser can
   bypass the BFF's SSRF guard entirely (finding S2) and finding S1 is credential leakage
   in the live path. If it is private, `CrawlAPIService` is broken in production instead.
   Check the environment files, the Firebase hosting config, and the BFF's deployment.
2. **Is `environment.CRAWL4AI_API_KEY` used anywhere that actually applies it?**
   `crawlapi.service.ts:29` contains `environment.CRAWL4AI_API_KEY !== '' ? url : url` —
   both branches identical. Determine whether the key is meant to be used or is dead.
3. **Does arachnefly re-validate redirects on the crawl path?** (finding S3)
4. **Do the frontend's `CrawlResult` / `CrawlStatus` types have any representation of
   "the site blocked us"?** (finding F4) Read `src/app/core/types/crawl-result.type.ts`
   and the crawl page components.
5. **What does the existing crawl test coverage look like?** `bff` has `bun test`; is
   there a spec for `crawlapi.service.ts` or the crawl components? Do not break what
   exists; add where it is missing.

Write your answers down. Then post a short plan and wait for confirmation before Phase 1
if any answer contradicts this prompt.

---

## Phase 1 — Security fixes (highest priority, independently shippable)

**1.1 Stop presenting the user's session token to crawl4ai.**
`crawlapi.service.ts:47-50` sends `'api-key': \`Bearer ${this.authService.token}\`` and
`'Authorization': \`Bearer ${this.authService.token}\`` on a request the browser makes to
`environment.API_CRAWL4AI_URL`. The caller's token must not leave our trust boundary.
Move the crawl credential to the BFF (it is already a secret there) and have the browser
authenticate to **our** route, not to crawl4ai.

Acceptance: no crawl request originating in the browser carries `authService.token` to a
non-deepscrape origin. Prove it by showing the request headers and the code path.

**1.2 Ensure there is exactly one guarded door.**
`functions/src/handlers/ai_handler.ts:66-84` guards `/api/crawl` with a scheme allowlist,
an RFC1918/link-local/metadata `BLOCKED_HOST` regex, a 2048-char cap, and `.slice(0, 50)`.
But the direct browser path bypasses it. Either route the client through the guarded BFF
route, or make the crawler itself refuse private targets. Two doors, one guarded, is not a
guard.

Acceptance: enumerate every path from the browser to a crawl engine and show each is
guarded. Take the guard logic and put it in **one** shared module — do not copy the regex.

**1.3 Close the redirect hole, or document precisely why you cannot.**
The author of that guard already wrote, at `ai_handler.ts:61-65`, that the check is
pre-redirect only and that re-checking after redirects is *"the half this repo cannot do
for it."* They were right: enforcement belongs where the connection is made. If
arachnefly or crawl4ai can be made to not follow redirects, or to re-validate the target
after each hop, do it there. If it genuinely cannot be fixed in this repo, do not paper
over it — leave the `ponytail:` comment accurate, and write the residual risk into
`docs/` with the reason.

There is precedent for doing this properly: the `x-set-cookie` passthrough was removed
from the same file to close a session-fixation issue (`ai_handler.ts:103-105`).

**1.4 Resolve the `CRAWL4AI_API_KEY` no-op.**
Either apply the key correctly or delete the dead ternary at `crawlapi.service.ts:29`. A
silent no-op reads as configuration that works.

Do not proceed to Phase 2 with a failing build or failing tests.

---

## Phase 2 — Make failure honest and the transport correct

**2.1 Add a `challenge` concept end to end.** (the product win)
Today the UI cannot tell *"this site blocked us"* from *"this site returned nothing."*
webrain returns a `challenge` field on every navigation
(`cloudflare_challenge` / `blocked` / `captcha`) for exactly this reason — see
`docs/concepts/challenges.mdx` in webrain.

- Extend the crawl result types with a challenge/blocked discriminator.
- Surface it in the crawl UI as its own state with a clear next action (retry with a real
  browser session, or attach one of the user's existing browser profiles — the
  `BrowserProfiles` collection already exists).
- Never render a challenge or consent page as if it were scraped content.

Acceptance: a blocked crawl and an empty crawl produce visibly different UI, and both are
covered by a test.

**2.2 Restore streaming on the BFF.**
`ai_handler.ts:117` buffers the whole crawl response with `await apiResponse.arrayBuffer()`
and then `res.end(Buffer.from(buffer))`. The correct implementation is already written and
commented out in the same file (`handleStreamedApiResponse` at lines 25-57, using
`streamPipeline(apiResponse.body, res)`), with matching dead imports at lines 5-6 and 10
and a commented call at line 124. Restore it, then delete the dead code.

Acceptance: a large crawl streams without materialising the whole body in memory, and the
dead block and its imports are gone.

**2.3 Return a real content-type.**
The handler sets `Content-Type: application/octet-stream` (`ai_handler.ts:119`), forcing
the client to `JSON.parse(arrayBufferToString(response))` with
`responseType: 'arraybuffer'` (`crawlapi.service.ts:60`). Send JSON as JSON. Keep the
binary passthrough only where the payload genuinely is binary.

**2.4 Replace the string-surgery endpoint derivation.**
`crawlapi.service.ts:78` builds the job endpoint via
`this.crawl4AiEndpoint.replace('/crawl', '/job')`. Use explicit constants.

---

## Phase 3 — Frontend quality (frontend-only, no platform dependency)

Pick these up only after Phases 1-2 are green. They are independent of the platform work
and each should be its own commit.

**3.1 Zoneless change detection.** `zone.js ~0.15.1` is still a runtime dependency
(`package.json`). Angular 20 supports `provideZonelessChangeDetection()`. This is a
Firestore-heavy dashboard — precisely the workload that benefits. Migrate in steps, and
measure before/after rather than assuming.

**3.2 One styling system.** Tailwind 3.4 coexists with 108 SCSS files. Two systems means
two sources of truth for spacing, colour and breakpoints. Do not attempt a big-bang
rewrite; propose a containment rule (new UI uses one system) and convert opportunistic
files. Tailwind 4 is available but is a CSS-first rewrite — treat it as a separate
decision, not a drive-by upgrade.

**3.3 De-duplicate env parsing.** `src/config/env.ts`, `functions/src/config/env.ts` and
a further `config/` copy each parse env independently and have already drifted. Extract
one shared parser, or document why three are required.

For all of Phase 3: **keep public API and user-visible behaviour identical unless the task
says otherwise**, and run the existing test suites before and after.

---

## Phase 4 — Wire to webrain through arachnefly

Only start this once Phase 1 is merged. Read
`Rust/webrain/docs/DEEPSCRAPE_FRONTEND_HANDOFF.md` §3-4 and
`Rust/webrain/docs/PLATFORM_EXPOSURE_PLAN.md` first — the target architecture, the
transport guards and their verification are all specified there.

The intended shape:

```
Angular ──► BFF /api/crawl ──► arachnefly ──► webrain worker (private 6PN) ──► typed JSON + challenge
```

**What webrain gives you**, so you do not rebuild it: typed JSON extraction
(`extract_json`, `autoschema`, `table`, `bm25`) instead of shipping raw markup to the
client; the `challenge` signal; `vault` + profiles + TOTP binding to your existing
`BrowserProfiles`; `webrain_batch` with `concurrency` and `cdp_urls` for parallelism;
`validate_urls` for redirect-safe target validation.

**Constraints — violating these is a defect, not a tradeoff:**

1. **webrain is read-only for you.** It stays generic by design: no per-site recipes, no
   hardcoded selectors, no intent sniffing. Anything deepscrape-specific belongs in
   deepscrape's adapter or in arachnefly. If you believe webrain must change, write up the
   gap and escalate rather than editing it.
2. **Set `WEBRAIN_NO_STATE=1` on shared workers.** Without it, the vault and browser
   profiles are process-global, so one tenant can read and then log into another tenant's
   accounts. With it, shared workers host stateless tenants only. Verified behaviour and
   the probe script are in webrain's `docs/PLATFORM_EXPOSURE_PLAN.md` §7.
3. **Never share a `WEBRAIN_PROXIES` pool across tenants.** It is process-global and
   rotates on 403/429, so one tenant hitting a wall rotates another tenant's traffic.
   Per-tenant lanes are unbuilt.
4. **Only real routes may fall through to the CSR shell.** deepscrape already enforces
   this (`bff/public.spec.ts` pins it); keep it true for any new route.
5. **CSRF stays enforced on new `/api` routes.**
6. **A challenge page is not a successful scrape.** Read the `challenge` field on every
   response and never extract a challenge/login/consent page as target content.

---

## Definition of done

- Phase 0 answered with evidence, including the production reachability question.
- Phase 1 shipped: no user token reaches crawl4ai; exactly one guarded ingress with the
  guard shared, not copied; redirect handling either fixed or documented with its
  residual risk; the `CRAWL4AI_API_KEY` no-op resolved.
- Phase 2 shipped: `challenge` is a first-class state with a test proving blocked ≠ empty;
  BFF streams; content-type is truthful; dead code removed.
- `npm run build`, `cd bff && bun test` and the Angular test suite pass.
- `mcp_codebase-memo_index_repository` re-run for `deepscrape` with the new node/edge counts reported.
- Every claim in your summary is backed by a command you ran. If you did not run it, say
  so plainly.
- Nothing was deployed.

## Anti-patterns — do not do these

- **Do not add per-site handling to webrain** (or to deepscrape's shared crawl path) to
  make one domain work. The LLM's prompt is the only source of intent.
- **Do not return raw HTML to the client.** Keep markup out of the context window and out
  of the browser payload; return typed JSON.
- **Do not copy the `BLOCKED_HOST` regex into a second place.** Share it.
- **Do not treat a browser as disposable.** Profile, session and challenge are runtime
  state. The fix for a protected site is real Chrome plus a persistent profile plus a
  session — never "discard the browser and retry anonymously."
- **Do not describe header spoofing as stealth.** Parity is not stealth.
- **Do not claim a fix without a runnable check.** A commented-out implementation is not a
  fix; neither is a green build with no test exercising the new path.
- **Do not deploy.**
