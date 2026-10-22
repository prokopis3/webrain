#!/usr/bin/env bash
# Bundle the webrain skill into a single-top-level-dir zip for claude.ai upload.
#
#   bash build-skill.sh   ->  dist/webrain.skill  (zip, top-level dir "webrain/")
#
# The bundle is exactly what is committed under skills/webrain (SKILL.md,
# references/, workflows/, prompts/, evals/, credentials/, scripts/).
# `git archive` of a committed dir IS the layout contract, so there is no
# second allowlist to keep in sync as the skill grows.
set -euo pipefail

# unzip is NOT guaranteed present. Git Bash on Windows does not ship it, and a
# PowerShell `unzip` on PATH shadows it with a different CLI — which is how this
# script used to fail with a misleading "SKILL.md missing from bundle" instead
# of naming the missing tool. Check before doing any work.
if ! command -v unzip >/dev/null 2>&1; then
  echo "error: unzip not found — required to validate the archive." >&2
  echo "  Windows: run this from WSL, or install unzip for Git Bash." >&2
  exit 1
fi

# Repo guard FIRST (before any git command) so running outside a git work tree
# prints this friendly message, not a raw `fatal` from git status.
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || ROOT=""
if [ -z "$ROOT" ]; then
  echo "not a git clone — git archive needs the repo (commit the skill first)" >&2
  exit 1
fi

# git archive MUST run from the repo ROOT. Run from inside skills/webrain,
# `git archive HEAD:skills/webrain` silently produced an archive containing only
# the `webrain/` directory entry and no files at all — which is what this script
# used to report as "SKILL.md missing from bundle". Verified: the identical
# command from the repo root yields every entry.
cd "$ROOT"
SKILL_REL="skills/webrain"
OUT_REL="$SKILL_REL/dist/webrain.skill"
mkdir -p "$SKILL_REL/dist"
# ponytail: git archive only — no Python fallback. Needs a committed skill dir.
# git archive packages HEAD — warn if the working tree has newer changes that
# would be silently excluded from the bundle. We're already in skills/webrain,
# so scope the status to `.` and exclude the dist output dir (just created
# above, otherwise the warning fires on every clean run).
if git status --porcelain -- "$SKILL_REL" ":(exclude)$SKILL_REL/dist" | grep -q .; then
  echo "warning: uncommitted changes in skills/webrain are NOT included in the bundle" >&2
fi

# Don't swallow git archive's real failure (2>/dev/null masked every error as
# "not a git clone").
if ! git archive --format=zip --prefix=webrain/ --output="$OUT_REL" HEAD:"$SKILL_REL"; then
  echo "git archive failed — is skills/webrain committed at HEAD?" >&2
  exit 1
fi

# Integrity only. The old layout allowlist ("one SKILL.md + scripts/, nothing
# else") was written when the skill was two files, and silently became a rule
# the skill could never satisfy once references/, workflows/, prompts/, evals/
# and credentials/ were added — `git archive` already packages exactly what is
# committed, so a hand-maintained copy of that list can only ever drift.
unzip -t "$OUT_REL" >/dev/null || {
  echo "invalid archive produced" >&2
  exit 1
}
if ! unzip -Z1 "$OUT_REL" | grep -qx 'webrain/SKILL\.md'; then
  echo "SKILL.md missing from the bundle — is skills/webrain/SKILL.md committed at HEAD?" >&2
  exit 1
fi

echo "wrote $OUT_REL"
