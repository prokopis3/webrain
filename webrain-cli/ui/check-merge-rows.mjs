// The merge that puts a tab's missing history in front of the rows it already
// streamed, checked without a browser. `missingHead` is the whole decision: a
// tab that reloaded during a turn holds only the turn's tail, and the count that
// says how much is missing comes from the server's message log (`total`) rather
// than from the stored rows, which a second tab may be behind on.
//
//   node check-merge-rows.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");
const start = src.indexOf("  function missingHead(");
const end = src.indexOf("\n  }", start);
assert.ok(start > 0 && end > start, "missingHead not found in App.svelte");
const missingHead = new Function(
  src.slice(start, end + 4) + "\n  return missingHead;",
)();

const u = (n) => ({ id: n, kind: "user", text: `q${n}` });
const a = (n) => ({ id: n, kind: "assistant", text: `a${n}` });
const tool = (n) => ({ id: n, kind: "tool", text: "search" });

// First sight of a conversation: no rows, so the stored copy is all of it.
assert.deepEqual(missingHead([u(1), a(1)], [], 2), [u(1), a(1)]);
// The reported bug: the tab joined during turn 2, so it holds the answer to a
// question it never saw. The question — and the turn before it — go in front.
assert.deepEqual(missingHead([u(1), a(1), u(2)], [a(2)], 4), [u(1), a(1), u(2)]);
// A pane that is already whole gets nothing. This is what stops the merge from
// duplicating a thread every time a turn ends.
assert.deepEqual(missingHead([u(1), a(1)], [u(1), a(1)], 2), []);
// A pane that holds more messages than the log cannot be behind it.
assert.deepEqual(missingHead([u(1)], [u(1), a(1), u(2), a(2)], 2), []);
// Rows that hang off a message travel with it, so a tool call the tab missed is
// not silently dropped from the middle of the prefix.
assert.deepEqual(missingHead([u(1), a(1), tool(2), u(2)], [a(2)], 4), [
  u(1),
  a(1),
  tool(2),
  u(2),
]);
// The stored transcript can itself be one turn behind (a save that has not
// landed). Hand back what exists rather than inventing rows — the count is the
// only thing this function is allowed to trust.
assert.deepEqual(missingHead([u(1)], [a(2)], 3), [u(1)]);
// A note is not a message: counting rows instead of messages would make this
// look complete and leave the question out.
assert.deepEqual(missingHead([u(1), a(1)], [{ id: 9, kind: "note" }], 2), [
  u(1),
  a(1),
]);

// The call sites the merge depends on. `seedLive` has to refuse while another
// thread is on screen (its rows belong to that thread) and has to be retried
// once the turn ends, or a tab that reloaded mid-turn never comes back.
assert.ok(src.includes("if (!current || busy || viewing) return;"), "seedLive runs while viewing");
assert.ok(src.includes("if (rowsFor !== current) {"), "the idle handler does not retry the seed");
assert.ok(
  src.indexOf("cache.delete(current);") > src.indexOf('case "assistant.idle":'),
  "the seed reads a cache that predates the turn",
);
// `rowsFor` is what the transcript save trusts, and it is set only after the
// merge — a partial pane that claimed the conversation would replace a complete
// transcript with its tail.
const claim = src.indexOf("    rowsFor = current;", src.indexOf("async function seedLive("));
const merge = src.indexOf("missingHead(disk, items", src.indexOf("async function seedLive("));
assert.ok(claim > merge, "seedLive claims the conversation before the pane is whole");

// A conversation is a session, so the state that belongs to one must not follow
// you into another: the draft you were typing, the runtime's queue of prompts
// behind the running turn, the cost panel's "this turn", and the directory an
// attachment was uploaded under. Each of these was on the page, not the thread.
assert.ok(src.includes("function swapDraft(next) {"), "no per-conversation draft");
const swaps = (src.match(/swapDraft\(/g) || []).length;
assert.equal(swaps, 4, "swapDraft must be defined once and called from all three thread changes");
assert.ok(src.includes("{#if queued.length && !past}"), "the queue is shown while reading another thread");
assert.ok(src.includes("if (turnUse && !past) {"), "the cost panel shows the live turn while reading another");
assert.ok(
  src.includes("encodeURIComponent(a.convo || current)"),
  "an attachment is addressed by the thread on screen, not the one it was uploaded under",
);

console.log("merge-rows: 16 assertions passed");
