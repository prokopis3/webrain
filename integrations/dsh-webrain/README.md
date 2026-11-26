# webrain for DeepSeek Harness

A [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (`dsh`)
bundle that connects [webrain](https://github.com/prokopis3/webrain)'s Model
Context Protocol server. Every webrain tool becomes callable by the agent as
`mcp__webrain__<tool>` — 19 tools for real-Chrome navigation, challenge-aware
scraping, extraction, crawling, screenshots, PDFs, and search.

```text
webrain MCP (stdio: `webrain mcp`)
        │  tools/list, tools/call
        ▼
@deepseek-ai/dsh-mcp-client  →  ctx.tools  →  mcp__webrain__webrain_navigate
```

## Requirements

- DeepSeek Harness with the `@deepseek-ai/dsh-mcp-client` package. Every shipped
  profile provides it.
- The `webrain` binary on the `PATH` of the process that starts the harness:
  - Windows: `scoop install webrain`
  - macOS: `brew install prokopis3/webrain/webrain`
  - anywhere else: a binary from
    [Releases](https://github.com/prokopis3/webrain/releases)
  - `webrain --version` must work from the shell that starts the harness.
- A browser engine, once: `webrain install` downloads Chrome for Testing into
  webrain's engine cache. Without it, navigation fails with an engine error even
  though the tool list is complete. `webrain doctor` diagnoses both.

## Install

In the Web UI: **Plugins** → **Add plugin**, paste a spec from below or choose
the bundle directory. From a shell:

```bash
# The @ixnk scope on the npm registry, once the release workflow published it:
dsh plugin --profile <profile> add @ixnk/dsh-webrain

# A pinned commit of this repository. pnpm 11 or newer is required for a Git
# subdirectory install; the parameters are separated with npm's "::" form
# because pnpm 11 rejects the "&path:" spelling it documents.
dsh plugin --profile <profile> add \
  'github:prokopis3/webrain#<sha>::path:integrations/dsh-webrain'

# A checkout on this machine:
dsh plugin --profile <profile> add 'D:\path\to\webrain\integrations\dsh-webrain'
```

`dsh plugin` forwards its arguments to pnpm, so the verb is `add` (not
`install`), and installing a package into a profile that is a pnpm workspace
root can need `ignore-workspace-root-check=true` in
`$DSH_HOME/profiles/<profile>/.npmrc`. A live profile picks the bundle up
without a restart.

## What appears

- A bundle card named **webrain browser tools**, with its own icon and the
  plugin name `webrain`.
- One enabled row: **MCP client** (row id `webrain-mcp`, `serverName:
  webrain`). `@deepseek-ai/dsh-mcp-client` owns that row's title and generic
  artwork — the webrain icon belongs to the bundle card.
- The tools `mcp__webrain__webrain_navigate`, `mcp__webrain__webrain_extract`,
  `mcp__webrain__webrain_batch`, `mcp__webrain__webrain_observe`,
  `mcp__webrain__webrain_guide`, and the rest of webrain's 19-tool surface.

## Verify

1. **Plugins** shows the *webrain browser tools* bundle and one enabled
   **MCP client** row.
2. Ask the agent to call `mcp__webrain__webrain_guide`. It returns webrain's
   browser/challenge/extraction decision guide.
3. Then a real task:

   > Call `mcp__webrain__webrain_guide`, then open `https://example.com` and
   > extract the heading.

## Configuration

The bundle inserts a row with id `webrain-mcp`. Override it with an
`id`-targeted patch in the profile's `cordis.patch.yml`
(`$DSH_HOME/profiles/<profile>/cordis.patch.yml`); restate the whole `config`,
because a patch replaces it wholly. With `patchReload: live` the change applies
without a restart:

```yaml
- id: webrain-mcp
  name: '@deepseek-ai/dsh-mcp-client'
  config:
    serverName: webrain
    transport: stdio
    command: '/opt/homebrew/bin/webrain'   # absolute path for GUI launches
    args: ['mcp']
    env:
      WEBRAIN_STEALTH_NOISE: '0'           # the bundle default; '1' restores noise
      CDP_URL: 'http://127.0.0.1:9222'     # reuse an already-open Chrome
      WEBRAIN_PROFILES_DIR: '/Users/me/Library/Application Support/webrain/profiles'
    toolCallTimeoutMs: 600000              # default from the bundle: 300000
    startupTimeoutMs: 60000
    failOnStartupError: true
```

An absolute `command` matters on macOS when the harness is launched from
Finder: a GUI process sees a minimal `PATH`, so a Homebrew `webrain` is
invisible.

The bundle starts webrain with `WEBRAIN_STEALTH_NOISE=0`: the fingerprint-noise
block is off, the same choice `webrain chat --serve` makes for itself. It is the
documented fix for a page that renders but never mounts (an SPA grid stays empty
while a hand-opened tab of the same URL works). Set it to `'1'` to restore the
noise shim, for example to reproduce a stealth-related report.

One `serverName` can be registered once per scope. If you also keep a
hand-written `webrain` MCP entry (for example the HTTP form,
`webrain mcp --http 9223`), disable or remove it — otherwise the bundle's row
fails with `mcp-client: serverName "webrain" is already in use by another
mcp-client instance`.

## What it touches

- The harness spawns one `webrain mcp` child process and speaks JSON-RPC over
  its stdio. webrain's logs go to stderr.
- webrain keeps its own browser profiles and engine cache in the platform data
  and cache directories (`…/webrain/profiles`, `…/webrain/browsers`),
  overridable with `WEBRAIN_PROFILES_DIR` / `WEBRAIN_BROWSERS_DIR`;
  `webrain help env` lists every variable. Nothing is stored in the harness
  profile.
- Browsers, cookies, and screenshots stay on this machine. Pages are fetched
  only when a tool asks for a URL.
- Uninstalling the bundle, or turning it off, unregisters the tools and stops
  the child process. It does not delete webrain's browser profiles or engines.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| The **MCP client** row fails, no `mcp__webrain__*` tools | `webrain` is not on the harness's `PATH` | set an absolute `command`, or start the harness from a shell where `webrain --version` works; the Host log names the spawn failure |
| `mcp-client: serverName "webrain" is already in use by another mcp-client instance` | another MCP row uses `serverName: webrain` | disable or remove that row |
| `'pnpm' is not recognized` while installing | the harness process has no `pnpm` | install pnpm for the user that runs the harness, then retry |
| `ERR_PNPM_ADDING_TO_ROOT` while installing | the profile is a pnpm workspace root and pnpm refuses the add | add `ignore-workspace-root-check=true` to the profile's `.npmrc`, then retry |
| Tool calls time out on heavy pages | the default call budget is below the page's cost | raise `toolCallTimeoutMs` on the `webrain-mcp` row |
| Navigation fails with a missing browser/engine | `webrain install` never ran | run `webrain install`, then `webrain doctor` |
| `webrain_navigate` returns `challenge` | the page is gated | use real Chrome: `webrain launch <service> <profile> <url>` then `webrain login`, or point `CDP_URL` at that Chrome |

## Related

- `docs/mcp/deepseek-harness.mdx` — the user-facing guide in the webrain docs.
- `skills/webrain/SKILL.md` — webrain's operator skill for any MCP client.
- `docs/AGENT_DECISION_GUIDE.md` — which webrain tool for which task.

## License

MIT, the same license as webrain.
