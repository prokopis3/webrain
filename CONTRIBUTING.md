# Contributing to webrain

Thanks for contributing.

## Issues vs pull requests

If the problem is easy to reproduce, prefer opening a GitHub issue. A clear
issue with reproduction steps, expected behavior, actual behavior, logs,
screenshots, or traces is usually the fastest path to a fix.

Pull requests are more useful when the problem depends on an environment you
may not have — Windows/macOS-specific behavior, unusual shells, GPU/display
setups, browser providers, or other local configuration. In those cases a PR
captures the behavior in the environment where it actually occurs.

## Development setup

- **Rust 1.85+** (edition 2024). Install via [rustup](https://rustup.rs).
- Workspace: `webrain-core` (CDP client, engines, vault, launch, install) ·
  `webrain-mcp` (MCP server + tool schemas) · `webrain-cli` (single binary).
- Build / check / test:
  ```bash
  cargo build --workspace
  cargo test --workspace
  cargo fmt --all --check
  cargo clippy --workspace --all-targets
  ```
  CI runs `fmt` + `clippy` + `test` on every push to `main`, so a green local
  run is the bar.

## Pull request policy

Pull requests are welcome and encouraged. Keep them focused and easy to review
independently.

- **Conventional Commits** — `<type>(<scope>): <subject>`. Valid types: `feat`,
  `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`,
  `revert`. Valid scopes (mirror the crates/subsystems): `core`, `engines`,
  `mcp`, `tools`, `cli`, `install`, `launch`, `login`, `vault`, `vision`,
  `pdf`, `download`, `cookies`, `batch`, `extraction`, `navigation`,
  `stealth`, `antibot`, `session`, `media`, `docs`, `build`, `ci`, `style`,
  `perf`, `dist`, `deps`, `config`, `test`, `skill`, `script`, `release`.
  See `commitlint.config.js`.
- **CHANGELOG.md is mandatory.** If you change Rust source under
  `webrain-core/`, `webrain-mcp/`, or `webrain-cli/`, add an entry under
  `[Unreleased]` in `CHANGELOG.md` (Keep a Changelog format). The
  `Changelog Enforce` CI check **fails the run** if source changed without a
  changelog entry, so this is not optional.
- Most PRs are treated as proposals or references rather than changes merged
  verbatim. The maintainer may reimplement a change to own its assumptions and
  failure modes — the submitted code still helps as a reproduction or design
  reference.

## What makes a great PR

- a clear description of the problem being solved
- a minimal reproduction or failing test when possible
- notes about edge cases and tradeoffs
- focused changes that are easy to review independently
- any relevant logs, screenshots, traces, or benchmarks

## License

By contributing you agree your work is licensed under the [MIT License](LICENSE).
