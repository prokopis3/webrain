#!/usr/bin/env bash
# Bundle the webrain skill into a single-top-level-dir zip for claude.ai upload.
# Mirrors the claude-video `watch` packaging: one SKILL.md + scripts/, nothing else.
#
#   bash build-skill.sh   ->  dist/webrain.skill  (zip, top-level dir "webrain/")
set -euo pipefail
cd "$(dirname "$0")/.."          # skills/webrain

# Repo guard FIRST (before any git command) so running outside a git work tree
# prints this friendly message, not a raw `fatal` from git status.
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "not a git clone — git archive needs the repo (commit the skill first)" >&2
  exit 1
fi

mkdir -p dist
# ponytail: git archive only — no Python fallback. Needs a committed skill dir.
# git archive packages HEAD — warn if the working tree has newer changes that
# would be silently excluded from the bundle. We're already in skills/webrain,
# so scope the status to `.` and exclude the dist output dir (just created
# above, otherwise the warning fires on every clean run).
if git status --porcelain -- . ':(exclude)dist' | grep -q .; then
  echo "warning: uncommitted changes in skills/webrain are NOT included in the bundle" >&2
fi

# Don't swallow git archive's real failure (2>/dev/null masked every error as
# "not a git clone").
if ! git archive --format=zip --prefix=webrain/ --output=dist/webrain.skill HEAD:skills/webrain; then
  echo "git archive failed — is skills/webrain committed at HEAD?" >&2
  exit 1
fi

# Integrity + layout: the contract is one SKILL.md + scripts/, nothing else.
unzip -t dist/webrain.skill >/dev/null || { echo "invalid archive produced"; exit 1; }
# grep -vE exits 1 when NO lines match (the happy path) — tolerate that under set -e.
EXTRA=$(unzip -Z1 dist/webrain.skill | grep -vE '^webrain/$|^webrain/SKILL\.md$|^webrain/scripts/' || true)
# The layout check must also verify the required entries are PRESENT.
if ! unzip -Z1 dist/webrain.skill | grep -qx 'webrain/SKILL.md'; then
  echo "SKILL.md missing from bundle" >&2
  exit 1
fi
if [ -n "$EXTRA" ]; then
  echo "unexpected files in bundle: $EXTRA" >&2
  exit 1
fi

echo "wrote dist/webrain.skill"
