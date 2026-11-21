// The follow-up chips under a turn, checked without a browser.
// `suggestNext` is a pure function of the transcript window: evidence first
// (a failed call, a host that answered, files that were written), then the
// generic follow-ups that are true of any finished answer. The point of these
// assertions is the second half — a turn with no tools at all must still end
// with something to press.
//
//   node check-next-steps.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");
const start = src.indexOf("  function suggestNext({");
const end = src.indexOf("\n  }", start);
assert.ok(start > 0 && end > start, "suggestNext not found in App.svelte");
// Self-contained on purpose: it carries its own host parser so it can be lifted
// out of the component like this and tested with no DOM.
const suggestNext = new Function(src.slice(start, end + 4) + "\n  return suggestNext;")();

const turn = (extra = {}) => ({
  shown: [{ kind: "user", text: "what changed in 0.17?" }, { kind: "assistant", text: "A few things." }],
  sources: [],
  busy: false,
  past: false,
  ...extra,
});

// Never while the turn is running, or on a read-only (past) transcript.
assert.deepEqual(suggestNext(turn({ busy: true })), [], "busy");
assert.deepEqual(suggestNext(turn({ past: true })), [], "past");
assert.deepEqual(suggestNext({ shown: [], sources: [], busy: false, past: false }), [], "empty");

// A plain question with no tools: the generic floor, and always a full set.
const plain = suggestNext(turn());
assert.equal(plain.length, 3, "a finished turn always offers three");
assert.ok(plain.includes("give me a concrete example"), "generic example chip");

// Evidence: the failed call comes first and names the wire tool.
const failed = suggestNext(
  turn({ shown: [{ kind: "assistant", text: "x" }, { kind: "tool", name: "webrain_scrape", status: "fail" }] }),
);
assert.equal(failed[0], "why did webrain_scrape fail?", "failure leads");

// Two sources are a comparison; one is a table from that host, www stripped.
assert.ok(
  suggestNext(turn({ sources: [{ url: "https://a.example/x" }, { url: "https://b.example/y" }] })).includes(
    "compare the top 3 sources",
  ),
  "two sources compare",
);
assert.ok(
  suggestNext(turn({ sources: [{ url: "https://www.example.com/p?q=1" }] })).includes(
    "extract the tables from example.com",
  ),
  "one source names the host",
);

// A capture and a write are different follow-ups.
assert.ok(
  suggestNext(turn({ shown: [{ kind: "assistant", text: "x" }, { kind: "tool", name: "webrain_observe_pixel", status: "ok" }] })).includes(
    "what does the capture actually show?",
  ),
  "capture",
);
assert.ok(
  suggestNext(turn({ shown: [{ kind: "assistant", text: "x" }, { kind: "tool", name: "write_file", status: "ok" }] })).includes(
    "show me the diff you made",
  ),
  "write",
);

// Answer shape decides the rest: code is explainable, length is summarisable.
assert.ok(
  suggestNext(turn({ shown: [{ kind: "assistant", text: "```js\n1\n```" }] })).includes("explain that code line by line"),
  "code",
);
assert.ok(
  suggestNext(turn({ shown: [{ kind: "assistant", text: "y".repeat(601) }] })).includes("summarise that in three bullets"),
  "long answer",
);

// Bounds: three at most, no duplicates, and no chip that repeats the question.
const crowded = suggestNext(
  turn({
    shown: [
      { kind: "user", text: "go deeper on the first point" },
      { kind: "tool", name: "webrain_scrape", status: "fail" },
      { kind: "tool", name: "write_file", status: "ok" },
      { kind: "assistant", text: "```js\n1\n```" + "y".repeat(700) },
    ],
    sources: [{ url: "https://a.example/x" }, { url: "https://b.example/y" }],
  }),
);
assert.ok(crowded.length <= 3, "capped at three");
assert.equal(new Set(crowded).size, crowded.length, "no duplicates");
assert.ok(!crowded.includes("go deeper on the first point"), "never echoes the prompt");
for (const s of crowded) assert.ok(!/undefined|null|NaN|\[object/.test(s), `clean chip: ${s}`);

// The model writes its own three, in a tail the page lifts out. `parseSuggest` is
// the whole contract: text without the tail, up to three prompts, markers gone.
const pStart = src.indexOf("  function parseSuggest(");
const pEnd = src.indexOf("\n  }", pStart);
assert.ok(pStart > 0 && pEnd > pStart, "parseSuggest not found in App.svelte");
const parseSuggest = new Function(src.slice(pStart, pEnd + 4) + "\n  return parseSuggest;")();

assert.deepEqual(parseSuggest("plain answer"), { clean: "plain answer", list: [] }, "no tail");
const p1 = parseSuggest("The answer.\n\n<suggest>a? | b? | c?</suggest>");
assert.equal(p1.clean, "The answer.", "tail removed");
assert.deepEqual(p1.list, ["a?", "b?", "c?"], "split on the bar");
assert.deepEqual(
  parseSuggest("<SUGGEST>1. do x | - do y | 3) do z</SUGGEST>").list,
  ["do x", "do y", "do z"],
  "markers stripped, tag case-insensitive",
);
assert.equal(parseSuggest("<suggest>a|b|c|d|e</suggest>").list.length, 3, "capped at three");
assert.deepEqual(parseSuggest("<suggest>  |  </suggest>"), { clean: "", list: [] }, "empty tail");

// The two hints are standing session instructions now, appended on the way to
// the server and stripped on the way back so the textarea only shows the
// operator's words. The round trip is the whole contract.
const hStart = src.indexOf("  const HTML_HINT =");
const hEnd = src.indexOf("\n  }", src.indexOf("  const stripHints ="));
assert.ok(hStart > 0 && hEnd > hStart, "hint helpers not found in App.svelte");
const { HTML_HINT, SUGGEST_HINT, withHints, stripHints } = new Function(
  src.slice(hStart, hEnd + 4) + "\n  return { HTML_HINT, SUGGEST_HINT, withHints, stripHints };",
)();

const suggestOnly = withHints("keep it short", false);
assert.ok(suggestOnly.startsWith("keep it short"), "the operator's words come first");
assert.ok(suggestOnly.includes(SUGGEST_HINT), "the suggestion tail is always on");
assert.ok(!suggestOnly.includes(HTML_HINT), "no html vocabulary in markdown mode");
assert.ok(withHints("", true).includes(HTML_HINT), "html mode carries the vocabulary");
assert.equal(stripHints(suggestOnly), "keep it short", "round trip keeps the operator's half");
assert.equal(stripHints(withHints("", false)), "", "nothing of the page's own survives a strip");
assert.equal(
  withHints(stripHints(withHints("x", true)), true),
  withHints("x", true),
  "idempotent: the hint is never doubled",
);
assert.equal(stripHints(undefined), "", "a missing setting is empty, not undefined");
assert.ok(!/undefined|NaN|null/.test(withHints(undefined, true)), "no placeholders");

console.log("next steps: 28 checks passed");
