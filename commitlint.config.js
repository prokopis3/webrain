// Conventional Commits for webrain — the canonical type + scope list.
// Types + scopes are enforced on PR titles by .github/workflows/pr-lint.yml;
// scopes mirror the real crates/subsystems (see CHANGELOG.md prefixes).
module.exports = {
  rules: {
    'type-enum': [2, 'always', [
      'feat', 'fix', 'docs', 'style', 'refactor', 'perf',
      'test', 'build', 'ci', 'chore', 'revert',
    ]],
    'scope-enum': [2, 'always', [
      // NOTE: the overlap with type-enum (docs/build/ci/style/perf/test) is
      // DELIBERATE — housekeeping commits like `docs(docs):` or `ci(ci):` are
      // valid and read clearly. Keep both lists in sync manually (they mirror
      // the scope list in AGENTS.md + the PR-lint scopes block).
      // code layers
      'core', 'engines', 'mcp', 'tools', 'cli',
      // core subsystems
      'install', 'launch', 'login', 'vault', 'vision',
      // feature areas
      'pdf', 'download', 'cookies', 'batch', 'extraction',
      'navigation', 'stealth', 'antibot', 'session', 'media',
      // housekeeping
      'docs', 'build', 'ci', 'style', 'perf', 'dist',
      'deps', 'config', 'test', 'skill', 'script', 'release',
    ]],
    // Scopes are mandatory (the copilot-instructions + this header require
    // `<type>(<scope>):`), so an empty scope is an error, not a warning.
    'scope-empty': [2, 'never'],
    'type-case': [2, 'always', 'lower-case'],
    'subject-empty': [2, 'never'],
    'header-max-length': [2, 'always', 100],
  },
};
