#!/usr/bin/env bash
# Bundle the webrain skill into a single-top-level-dir zip for claude.ai upload.
# Mirrors the claude-video `watch` packaging: one SKILL.md + scripts/, nothing else.
#
#   bash build-skill.sh   ->  dist/webrain.skill  (zip, top-level dir "webrain/")
set -euo pipefail
cd "$(dirname "$0")/.."          # skills/webrain
mkdir -p dist
# ponytail: git archive only — no Python fallback. Needs a committed skill dir.
git archive --format=zip --prefix=webrain/ --output=dist/webrain.skill HEAD:skills/webrain 2>/dev/null \
  || { echo "not a git clone — git archive needs the repo (commit the skill first)"; exit 1; }
echo "wrote dist/webrain.skill"
