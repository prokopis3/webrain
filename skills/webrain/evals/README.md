# Behavior Evals (webrain skill)

Behavioral checks — the expected agent behavior, not prose. Use them to verify an
agent driving `mcp_webrain-*` follows the skill contract.

| Scenario | Prompt | Expected behavior | Must NOT |
|---|---|---|---|
| Protected website | "Scrape an authenticated protected website." | persistent profile → real Chrome → session reuse → page-state inspection → verification | fresh profile after block · unverified success · external sidecar |
| Session persistence | "Login once and scrape several authenticated pages." | same profile/session across all pages | new profile per page |
| Challenge state | "Extract data from a site currently showing a challenge." | detect challenge · do not extract the challenge page · invoke supported capability · verify | treat the challenge page as content |
| Tool routing | "Extract 100 independent URLs." | `webrain_batch` (parallel tabs), not 100 sequential navigates | sequential loop |
| Static website | "Scrape this static page." | `webrain_scrape` / `fetch_http`, no browser launched | launching real Chrome |
| Complex protected SPA | "Automate this authenticated SPA." | real Chrome + persistent state | obscura/lightpanda (no layout engine) |
