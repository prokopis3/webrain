Use this as the **master Copilot Chat prompt**. It is deliberately strict: it tells Copilot to inspect the actual repository first, then redesign the information architecture, update the skills/docs, remove the legacy stealth path, and validate that the resulting system is internally consistent.

# Webrain — Complete Repository, Agent Skills & Documentation Architecture Upgrade

You are working directly inside the **Webrain** repository.

Repository:

`https://github.com/prokopis3/webrain`

Your task is to perform a **complete professional restructuring and upgrade of the Webrain repository's documentation, agent skills, agent instructions, architecture documentation, and knowledge organization**.

Do NOT treat this as a simple documentation rewrite.

This is an **information-architecture migration for an AI-agent-first browser automation platform**.

The goal is to make Webrain significantly easier for:

* Claude
* GitHub Copilot
* Cursor
* Codex
* MCP clients
* autonomous coding agents
* AI search/retrieval systems
* human developers

to understand, route correctly, and use without hallucinating capabilities.

---

# 1. FIRST: AUDIT THE ENTIRE REPOSITORY

Before changing anything, inspect the repository thoroughly.

Do not assume the current architecture.

Inspect at minimum:

```text
AGENTS.md
README.md
docs/
docs/docs.json
skills/
src/
crates/
packages/
scripts/
examples/
tests/
Cargo.toml
package.json
all MCP/tool definitions
all browser implementations
all profile/session implementations
all challenge/anti-bot implementations
```

Search the entire repository for:

```text
stealth
stealth_solve
captcha
CAPTCHA
Cloudflare
Turnstile
anti-bot
antibot
challenge
profile
session
Chrome
real Chrome
browser
authentication
login
```

Build an internal understanding of:

1. current repository architecture
2. current browser architecture
3. current MCP tools
4. current profiles implementation
5. current sessions implementation
6. current authentication flow
7. current challenge detection
8. current anti-bot implementation
9. current extraction architecture
10. current agent skill architecture
11. current documentation architecture
12. current duplicated or contradictory information
13. obsolete documentation
14. claims that are stronger than the actual implementation

Do not modify files until this audit is complete.

---

# 2. CRITICAL PRODUCT DIRECTION

The old Webrain stealth-solve architecture is being removed.

The repository must NOT continue treating an external stealth solver/sidecar as the solution for protected websites.

The following concepts are legacy and must be removed from active architecture:

```text
stealth_solve.py
stealth solve
stealth-solve sidecar
external CAPTCHA solver as the Webrain architecture
"stealth solves CAPTCHA"
"stealth bypasses CAPTCHA"
```

Search the entire repository and remove or rewrite all active references.

If a reference exists only in historical changelog material, it may remain as historical information, but it must clearly be identified as legacy and must not appear as a recommended implementation.

Do not leave dead implementation paths simply because documentation no longer recommends them.

If the old implementation exists in the codebase and is genuinely obsolete according to the repository architecture, remove it safely and update imports/tests/build configuration accordingly.

---

# 3. NEW WEbrain CORE ARCHITECTURE

The new architecture must treat browser identity and browser state as first-class concepts.

The canonical protected-site model is:

```text
persistent profile
        ↓
real Chrome
        ↓
persistent session
        ↓
navigate
        ↓
inspect page state
        ↓
content / authentication / challenge / error
        ↓
handle state
        ↓
verify
        ↓
extract
        ↓
return structured result
```

The key architectural principle is:

> Browser profile + session + browser state are part of the execution state of a protected-site workflow.

Do NOT treat a browser as disposable.

Do NOT recommend:

```text
anonymous request
→ blocked
→ fresh browser
→ fresh profile
→ retry
```

Instead:

```text
profile
→ real Chrome
→ establish/restore session
→ navigate
→ preserve identity/state
→ handle page state
→ continue
```

---

# 4. REAL CHROME REQUIREMENT FOR PROTECTED WEBSITES

The documentation and agent skills must explicitly establish:

For unknown, authenticated, or protected websites, the preferred workflow is:

```text
persistent profile
→ real Chrome
→ persistent session
→ navigation
```

This must be clearly documented in:

```text
AGENTS.md
skills/webrain/SKILL.md
skills/webrain/references/core-rules.md
skills/webrain/references/browser-selection.md
skills/webrain/references/profiles.md
skills/webrain/references/sessions.md
skills/webrain/references/challenges.md
skills/webrain/workflows/protected-site.md
docs/agent/decision-guide.mdx
docs/agent/protected-sites.mdx
docs/concepts/profiles.mdx
docs/concepts/sessions.mdx
docs/concepts/challenges.mdx
```

Do not bury this information.

It is a core Webrain rule.

---

# 5. FUTURE CHALLENGE ENGINE

Webrain's long-term architecture is intended to move challenge handling into the Webrain runtime itself.

The conceptual architecture is:

```text
Real Chrome
    ↓
challenge detection
    ↓
challenge classification
    ↓
internal challenge engine
    ↓
local visual/interaction analysis where supported
    ↓
browser interaction
    ↓
challenge verification
    ↓
continue navigation
```

The implementation should be designed so that future challenge capabilities can be implemented internally, including Rust-based runtime/tool calls and local vision components where appropriate.

Use a clean abstraction such as:

```text
browser.challenge.detect
browser.challenge.inspect
browser.challenge.execute
browser.challenge.verify
```

or an equivalent API consistent with the existing Webrain MCP/tool architecture.

DO NOT hard-code the architecture around:

```text
stealth_solve
stealth sidecar
external CAPTCHA solver
```

The challenge engine should be an internal Webrain capability.

---

# 6. DO NOT HALLUCINATE CURRENT CAPABILITIES

There is an important distinction between:

```text
current implementation
```

and:

```text
future architectural goal
```

The documentation must never claim that Webrain currently solves every CAPTCHA unless the actual runtime implementation proves this.

Use capability-driven language.

For example:

GOOD:

> Webrain's architecture provides an internal challenge-handling subsystem designed to support protected-site workflows. Actual challenge capabilities depend on the runtime implementation and challenge type.

BAD:

> Webrain automatically bypasses every CAPTCHA.

Do not document future capabilities as already implemented.

If the repository already contains a working capability, verify it from source/tests before documenting it.

---

# 7. CREATE THE NEW AGENT SKILL ARCHITECTURE

Replace the monolithic skill structure with progressive disclosure.

Target:

```text
skills/
└── webrain/
    ├── SKILL.md
    │
    ├── references/
    │   ├── core-rules.md
    │   ├── architecture.md
    │   ├── tool-routing.md
    │   ├── browser-selection.md
    │   ├── profiles.md
    │   ├── sessions.md
    │   ├── authentication.md
    │   ├── challenges.md
    │   ├── extraction.md
    │   ├── pagination.md
    │   ├── crawling.md
    │   ├── batching.md
    │   ├── concurrency.md
    │   ├── troubleshooting.md
    │   └── capability-matrix.md
    │
    ├── workflows/
    │   ├── new-site.md
    │   ├── protected-site.md
    │   ├── authenticated-site.md
    │   ├── extraction.md
    │   ├── pagination.md
    │   ├── infinite-scroll.md
    │   ├── site-crawl.md
    │   └── batch.md
    │
    ├── evals/
    │   ├── evals.json
    │   ├── protected-site.json
    │   ├── session-persistence.json
    │   ├── extraction.json
    │   └── tool-selection.json
    │
    ├── scripts/
    └── assets/
```

Adapt this structure to the actual repository where appropriate.

Do not create meaningless files just to match the tree.

Each reference must have a clear purpose.

---

# 8. SKILL.md MUST BE A ROUTER

The main:

```text
skills/webrain/SKILL.md
```

must NOT become a giant documentation dump.

It should contain:

1. identity/purpose
2. mandatory rules
3. core browser-selection rules
4. protected-site rules
5. extraction rules
6. verification rules
7. routing table to references/workflows

Detailed knowledge must live in:

```text
references/
workflows/
```

The skill should use progressive disclosure.

The agent should be able to start with SKILL.md and load only the information relevant to the current task.

---

# 9. CORE-RULES REFERENCE

Create:

```text
skills/webrain/references/core-rules.md
```

It must contain the highest-priority operational rules.

At minimum:

```text
1. Browser state matters.
2. Profile state matters.
3. Session state matters.
4. Protected navigation starts with real Chrome.
5. Preserve browser identity.
6. A challenge page is not successful navigation.
7. CAPTCHA/challenge capability must be determined from runtime capability.
8. Never use the legacy stealth_solve architecture.
9. Never report an unverified bypass.
10. Verify final content before returning results.
```

Keep this concise and extremely explicit.

---

# 10. PROFILE ARCHITECTURE

Create:

```text
skills/webrain/references/profiles.md
docs/concepts/profiles.mdx
docs/agent/session-strategy.mdx
```

Explain:

* what a profile is
* what state it contains
* when to create one
* when to reuse one
* how profile persistence affects authentication
* how profile persistence affects protected-site workflows
* why a fresh profile can change browser/site state
* how agents should select profiles
* how profiles interact with sessions

Do not make unsupported claims about browser fingerprinting.

Document only behavior actually supported by Webrain.

---

# 11. SESSION ARCHITECTURE

Create:

```text
skills/webrain/references/sessions.md
docs/concepts/sessions.mdx
```

Define the session lifecycle:

```text
CREATE
 ↓
PROFILE ATTACHED
 ↓
REAL CHROME
 ↓
SESSION ACTIVE
 ↓
NAVIGATE
 ↓
PAGE STATE
 ↓
HANDLE
 ↓
EXTRACT
 ↓
VERIFY
 ↓
PRESERVE / TERMINATE
```

Explain:

* session creation
* persistence
* restoration
* authentication state
* browser/profile relationship
* failure/recovery
* session reuse across pages
* session isolation

Again: derive everything from the actual source implementation.

---

# 12. CHALLENGE ARCHITECTURE

Create:

```text
skills/webrain/references/challenges.md
docs/concepts/challenges.mdx
docs/reference/challenge-states.mdx
docs/architecture/challenge-runtime.mdx
docs/guides/protected-websites.mdx
```

Structure it around:

```text
challenge detection
challenge classification
challenge state
challenge capability
challenge execution
challenge verification
failure
human fallback
```

Do not reduce the architecture to "CAPTCHA solver".

A challenge is a browser/page state.

The agent must first detect and understand the state.

---

# 13. BROWSER SELECTION MUST BE STATE-AWARE

Do not structure browser selection only as:

```text
HTTP
Obscura
Lightpanda
Chrome
```

Use two dimensions:

```text
execution engine
+
browser/application state
```

For example:

```text
STATIC + PUBLIC
→ HTTP/static extraction

JAVASCRIPT + PUBLIC
→ lightweight browser

COMPLEX SPA
→ real browser

AUTHENTICATED
→ persistent profile/session

PROTECTED
→ persistent profile + real Chrome + session

PROTECTED + CHALLENGE
→ real Chrome + challenge capability
```

Create:

```text
skills/webrain/references/browser-selection.md
docs/agent/browser-selection.mdx
```

---

# 14. CREATE A CAPABILITY MATRIX

Create:

```text
skills/webrain/references/capability-matrix.md
docs/concepts/capability-matrix.mdx
```

The matrix must distinguish:

* static HTML
* JavaScript
* SPA
* authentication
* persistent profiles
* persistent sessions
* screenshots
* protected sites
* challenge detection
* challenge handling
* extraction
* crawling
* batching

Use:

```text
SUPPORTED
PARTIAL
RUNTIME-DEPENDENT
NOT SUPPORTED
```

Do not use optimistic checkmarks unless verified from the implementation.

---

# 15. PROTECTED-SITE WORKFLOW

Create:

```text
skills/webrain/workflows/protected-site.md
docs/guides/protected-websites.mdx
docs/recipes/protected-site.mdx
```

Canonical workflow:

```text
1. Identify target
2. Determine whether existing profile/session exists
3. Select persistent profile
4. Launch real Chrome
5. Restore/establish session
6. Navigate
7. Inspect page state
8. Detect challenge/auth/block state
9. Invoke supported internal capability if available
10. Verify target content
11. Extract
12. Validate result
13. Preserve session/profile state when appropriate
```

Explicitly prohibit:

```text
stateless first attempt
→ blocked
→ fresh browser
→ fresh profile
```

---

# 16. AGENT DECISION GUIDE

Rewrite the current decision guide around this hierarchy:

```text
USER TASK
 ↓
TARGET STATE
 ↓
PROFILE
 ↓
SESSION
 ↓
BROWSER ENGINE
 ↓
NAVIGATION
 ↓
PAGE STATE
 ↓
CHALLENGE / AUTH / CONTENT
 ↓
EXTRACTION
 ↓
VERIFICATION
```

The guide should answer:

> What should an agent do next?

not merely:

> What browser exists?

---

# 17. EXTRACTION ARCHITECTURE

Create:

```text
skills/webrain/references/extraction.md
docs/concepts/extraction.mdx
docs/guides/structured-extraction.mdx
```

Preferred strategy:

```text
autoschema
→ structured extraction
→ targeted DOM
→ JSON-LD
→ tables
→ raw HTML
```

Document:

* when each method should be used
* failure handling
* schema validation
* extraction verification
* avoiding challenge/consent/login page extraction

---

# 18. BATCHING / SCALE

Create:

```text
skills/webrain/references/batching.md
skills/webrain/references/concurrency.md
docs/guides/scraping-at-scale.mdx
```

Document:

* batch tool usage
* concurrency
* rate limiting
* profile isolation
* session isolation
* error aggregation
* retries
* partial failures

Do not encourage agents to create hundreds of sequential MCP calls when a Webrain batch primitive exists.

---

# 19. ADD ANTI-PATTERNS

Create:

```text
skills/webrain/references/anti-patterns.md
docs/guides/anti-patterns.mdx
```

Include examples such as:

```text
DO NOT:
- start protected workflows statelessly
- discard a working profile
- discard a working session
- switch to a fresh browser after a challenge
- treat challenge detection as successful scraping
- assume CAPTCHA handling exists without checking runtime capability
- use stealth_solve
- restore obsolete stealth architecture
- claim a bypass succeeded without verification
- extract challenge/login/consent pages as target content
- create sequential loops when batch tools exist
```

---

# 20. ADD AGENT EVALUATIONS

Create:

```text
skills/webrain/evals/
```

The evaluation suite should test behavior, not prose.

Minimum evaluations:

### Protected website

Prompt:

> Scrape an authenticated protected website.

Expected behavior:

```text
persistent profile
real Chrome
session reuse
page-state inspection
verification
```

Must NOT:

```text
stealth_solve
fresh profile after block
unverified success
```

### Session persistence

Prompt:

> Login once and scrape several authenticated pages.

Expected:

```text
same profile/session
```

### Challenge state

Prompt:

> Extract data from a site currently showing a challenge.

Expected:

```text
detect challenge
do not extract challenge page
invoke supported capability
verify
```

### Tool routing

Prompt:

> Extract 100 independent URLs.

Expected:

```text
batch execution
```

### Static website

Expected:

```text
do not unnecessarily launch real Chrome
```

### Complex protected SPA

Expected:

```text
real Chrome
persistent state
```

---

# 21. AGENTS.md

Rewrite:

```text
AGENTS.md
```

as the repository engineering contract.

It must cover:

```text
repository structure
architecture
coding rules
browser rules
profile rules
session rules
protected-site rules
challenge architecture
MCP/tool rules
testing
documentation
legacy paths
definition of done
```

At the very top include a short "Critical Rules" section.

The first rules should make it impossible for an agent to accidentally resurrect the old stealth architecture.

---

# 22. README

Rewrite README so that it is concise and professional.

Structure:

```text
Webrain
↓
What it is
↓
Why it exists
↓
Core architecture
↓
Capabilities
↓
Quickstart
↓
MCP integration
↓
Agent integration
↓
Architecture
↓
Documentation
↓
Development
```

Do not turn README into a giant manual.

Avoid unsupported marketing claims.

---

# 23. MINTLIFY / docs.json

Completely reorganize documentation navigation around user intent.

Use:

```text
Getting Started
Core Concepts
Agent
Guides
Recipes
Integrations
Reference
Architecture
Troubleshooting
```

Recommended navigation:

```text
Getting Started
- Introduction
- Installation
- Quickstart
- First Scrape

Core Concepts
- Architecture
- Browser Engines
- Profiles
- Sessions
- Authentication
- Challenges
- Extraction
- Capability Matrix

Agent
- Agent Overview
- Decision Guide
- Tool Routing
- Browser Selection
- Protected Sites
- Session Strategy
- Extraction Strategy
- Verification

Guides
- Protected Websites
- Authenticated Scraping
- Persistent Browser Profiles
- Structured Extraction
- Pagination
- Infinite Scroll
- Site Crawling
- Scraping at Scale

Recipes
- Protected Site
- Authenticated Site
- Persistent Session
- Dynamic SPA
- Tables
- Large Catalog

Integrations
- MCP
- Claude
- Cursor
- Copilot
- Codex

Reference
- Tools
- CLI
- Environment Variables
- Profiles API
- Sessions API
- Challenge States

Architecture
- Overview
- Browser Runtime
- Profile Runtime
- Session Runtime
- Extraction Runtime
- Challenge Runtime

Troubleshooting
- Blocked Site
- Session Lost
- Authentication Failed
- Challenge Failed
- Extraction Failed
- Browser Failed
```

Adapt the exact syntax to the current Mintlify configuration.

---

# 24. AI-FIRST DOCUMENTATION

Every important concept must have an answer-oriented page.

For example:

```text
docs/agent/protected-sites.mdx
```

should answer:

> How should Webrain handle a protected website?

`docs/agent/session-strategy.mdx`:

> How should Webrain preserve authentication and browser state?

`docs/agent/tool-routing.mdx`:

> Which Webrain tool should an agent call?

`docs/agent/verification.mdx`:

> How does Webrain know scraping actually succeeded?

Use:

* explicit rules
* decision trees
* tables
* short examples
* "when to use"
* "when not to use"
* failure states
* verification requirements

Avoid marketing language.

---

# 25. DOCUMENTATION CONSISTENCY

After rewriting the documentation, perform a repository-wide consistency audit.

Find contradictions such as:

```text
"stealth solves CAPTCHA"
vs
"CAPTCHA is unsupported"

"Chrome is optional"
vs
"Chrome is required for protected workflows"

"session is disposable"
vs
"session is persistent"

"profile is only cookies"
vs
"profile is browser state"
```

There must be exactly one canonical interpretation.

---

# 26. SEARCH/RETRIEVAL OPTIMIZATION

Optimize titles and descriptions for real developer/AI queries.

Pages should use titles like:

```text
Protected Websites
Persistent Browser Profiles
Browser Sessions
Webrain CAPTCHA and Challenge Handling
Agent Decision Guide
Webrain Browser Selection
Authenticated Web Scraping
Structured Web Extraction
Webrain MCP Tools
```

Descriptions should explicitly contain the concepts developers search for.

Avoid vague titles like:

```text
Advanced
Internals
How It Works
Deep Dive
```

unless necessary.

---

# 27. CROSS-LINK EVERYTHING

Every major MCP tool should link to:

```text
tool reference
→ relevant concept
→ relevant workflow
→ relevant recipe
→ troubleshooting
```

Every concept should link to:

```text
concept
→ agent guidance
→ implementation reference
→ recipe
```

Every workflow should link to:

```text
workflow
→ tools
→ concepts
→ troubleshooting
```

Avoid orphan pages.

---

# 28. CODE + DOCS MUST MATCH

This is mandatory.

When documentation describes a feature:

1. locate the implementation
2. verify its API
3. verify parameters
4. verify return values
5. verify errors
6. verify current naming
7. verify examples
8. update documentation accordingly

Never invent APIs merely because they look architecturally desirable.

For future challenge-engine APIs, clearly mark them as:

```text
planned
experimental
implemented
```

according to the actual repository state.

---

# 29. TEST EVERYTHING AFTER THE MIGRATION

Run the project's existing:

```text
cargo test
cargo check
cargo clippy
npm test
npm run build
documentation build
lint
format
```

Use only commands that actually exist in the repository.

Also validate:

```text
all links
all imports
all references
all skill files
all docs navigation
all MCP tool names
all examples
```

Search again for obsolete:

```text
stealth_solve
stealth-solve
stealth sidecar
```

There should be zero active references.

---

# 30. FINAL AUDIT REPORT

At the end, provide a concise report containing:

## Repository changes

List every created, modified, deleted, and moved file.

## Architecture changes

Explain the new:

```text
profile
session
real Chrome
page state
challenge engine
extraction
verification
```

relationship.

## Skill changes

Explain the new:

```text
SKILL.md
references/
workflows/
evals/
```

architecture.

## Documentation changes

Explain the new Mintlify navigation and major pages.

## Removed legacy architecture

Explicitly list everything removed from the stealth-solve model.

## Current vs future capabilities

Clearly separate:

```text
implemented
experimental
planned
```

## Validation

Report the exact tests/checks executed and their results.

## Remaining issues

Do not hide incomplete work.

---

# 31. IMPORTANT IMPLEMENTATION RULES

Do NOT:

* blindly create files without inspecting existing architecture
* duplicate the same documentation in ten places
* leave contradictory instructions
* preserve obsolete stealth-solve code just because it already exists
* claim unsupported CAPTCHA capabilities
* invent MCP tools
* invent APIs
* invent runtime behavior
* rewrite working source code unnecessarily
* break existing interfaces merely for documentation cleanliness
* make marketing claims stronger than the implementation
* create enormous SKILL.md files
* put all knowledge into AGENTS.md

DO:

* inspect first
* preserve working architecture
* refactor carefully
* remove genuinely obsolete stealth paths
* use progressive disclosure
* make agent routing explicit
* make browser state explicit
* make profile/session state explicit
* make challenge handling capability-driven
* verify documentation against source
* add behavioral evals
* validate the final repository

---

# 32. DEFINITION OF DONE

This task is complete only when:

```text
[ ] Repository audited
[ ] Existing architecture understood
[ ] Legacy stealth-solve architecture removed
[ ] No active stealth_solve references remain
[ ] Real Chrome protected-site workflow documented
[ ] Profile architecture documented
[ ] Session architecture documented
[ ] Challenge architecture documented
[ ] Internal challenge-engine architecture documented
[ ] SKILL.md converted into a router
[ ] references/ created
[ ] workflows/ created
[ ] evals/ created
[ ] AGENTS.md rewritten
[ ] README rewritten
[ ] Agent Decision Guide rewritten
[ ] Capability matrix created
[ ] Anti-patterns documented
[ ] Protected-site workflow created
[ ] Mintlify navigation reorganized
[ ] Agent-specific docs created
[ ] Human documentation created
[ ] Cross-links added
[ ] API/tool names verified against source
[ ] Documentation build passes
[ ] Code/tests pass
[ ] No contradictory anti-bot instructions remain
[ ] Current vs future capabilities clearly separated
[ ] Final repository audit completed
```

---

# 33. FINAL PRINCIPLE

The entire repository must converge on this mental model:

```text
                    USER TASK
                       │
                       ▼
                 AGENT DECISION
                       │
          ┌────────────┼────────────┐
          │            │            │
       PROFILE       SESSION      ENGINE
          │            │            │
          └────────────┼────────────┘
                       ▼
                  REAL BROWSER
                       │
                       ▼
                    NAVIGATE
                       │
                       ▼
                  PAGE STATE
                       │
       ┌───────────────┼────────────────┐
       │               │                │
    CONTENT          AUTH           CHALLENGE
       │               │                │
       │               │                ▼
       │               │        INTERNAL RUNTIME
       │               │                │
       └───────────────┴────────────────┘
                       │
                       ▼
                    VERIFY
                       │
                       ▼
                   EXTRACT
                       │
                       ▼
                 STRUCTURED RESULT
```

**Browser identity is state.**

**Profiles are state.**

**Sessions are state.**

**Challenges are page/runtime state.**

**Real Chrome is the preferred execution environment for protected workflows.**

**Challenge handling belongs inside Webrain's runtime architecture, not an external stealth sidecar.**

**Agents must verify outcomes instead of assuming them.**

Execute this migration directly in the repository. Do not merely propose the changes. Inspect → plan internally → implement → validate → audit → report.
