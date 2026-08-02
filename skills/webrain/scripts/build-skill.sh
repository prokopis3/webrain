#!/usr/bin/env bash
# Bundle the webrain skill into a single-top-level-dir zip for claude.ai upload.
# Mirrors the claude-video `watch` packaging: one SKILL.md + scripts/, nothing else.
#
#   bash build-skill.sh   ->  dist/webrain.skill  (zip, top-level dir "webrain/")
set -euo pipefail
cd "$(dirname "$0")/.."          # skills/webrain
mkdir -p dist
git archive --format=zip --prefix=webrain/ --output=dist/webrain.skill HEAD:skills/webrain 2>/dev/null \
  || python -m zipfile -c dist/webrain.skill SKILL.md scripts
echo "wrote dist/webrain.skill"
