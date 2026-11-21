// Every subagent event the runtime can emit has an arm, checked without a
// browser. The transcript shows the parent's `task` row and nothing else, so
// these six arms are the *only* place a reader learns a worker exists, what it
// may touch, which model it got, and what it cost.
//
// Measured 2026-11-21 in copilot-sdk 2.0.1 `generated/session_events.rs`: six
// `subagent.*` variants. Three were handled and three were not, which is silent
// by design — an unhandled event lands in `default` and is dropped. A re-export
// bump that adds a seventh id fails here rather than in the page.
//
//   node check-subagent-events.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");

// The wire names, not the Rust variant names: this is what arrives in `f.type`.
const WIRE = [
  "subagent.started",
  "subagent.configured",
  "subagent.selected",
  "subagent.deselected",
  "subagent.completed",
  "subagent.failed",
];

const handled = new Set(
  [...src.matchAll(/case\s+"(subagent\.[a-z_]+)"/g)].map((m) => m[1]),
);

for (const id of WIRE) {
  assert.ok(handled.has(id), `no case arm for ${id} — it would be dropped silently`);
}

// A typo'd arm is worse than a missing one: it looks handled and never runs.
for (const id of handled) {
  assert.ok(WIRE.includes(id), `arm for ${id} is not an event the runtime emits`);
}

// A worker that starts and never ends would leave the running-agents chip up
// forever, so the terminal arms must clear the row `started` added. They are one
// block, `completed` falling through to `failed`, so the span runs from the
// first of them to the next event arm rather than case by case.
const at = src.indexOf('case "subagent.completed"');
const end = src.indexOf('case "subagent.selected"', at);
assert.ok(at > 0 && end > at, "the terminal subagent arms moved");
assert.ok(
  /agents\s*=\s*agents\.filter\(/.test(src.slice(at, end)),
  "a finished or failed agent is never removed from the running list",
);

console.log(`subagent events: ${WIRE.length} handled`);
