# webrain docs

Source for the mintlify.com/webrain site. Deploys automatically from `main`.
Preview locally with `npx mintlify@latest dev`.

## Editing rules

- **One fact, one home.** Canonical numbers stay in sync everywhere — grep
  before you change one: **16 MCP tools**, **2 browserless engines**
  (duckduckgo · bing; google/brave need the connected CDP engine), **50 max
  results** per `webrain_serp` call (`limit` clamps `1..=50`).
- **Capability-truthful only.** Never claim behavior the runtime doesn't
  provide (AGENTS.md rule 5). SERP demos must reproduce today with
  `webrain serp "<query>" --engine duckduckgo` — `auto` merges unvalidated
  bing junk first, so it is not a showcase engine.
- **The guide mirrors the code.** `webrain-mcp/src/tools.rs::AGENT_GUIDE` must
  agree with `list_tools()` (16 registered). `webrain_eval_in_frame` is a
  hidden legacy executor — dispatch-only, never in `tools/list`.
- **Landing is sanitizer-aware.** The custom page (`index.mdx`) loads JS/CSS
  via jsDelivr; the Mintlify sanitizer strips `<canvas>`/`<circle>`/`<line>` —
  use DOM divs + CSS animation, and verify in the browser after edits.
- **User-facing change?** Add a `changelog.mdx` entry in the same PR.
