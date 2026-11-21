// The tool-row sentences, checked without a browser and without a framework.
// `stepTitle` is read out of App.svelte itself, so this cannot pass against a
// copy that no longer ships.
//
//   node check-step-titles.mjs        (from webrain-cli/ui)
//
// A renamed tool or a changed argument shows up here as a fallback sentence
// ("Read a file") instead of "Read undefined" in someone's transcript.
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");
const start = src.indexOf("  function stepTitle(it) {");
const end = src.indexOf("\n  }", start);
assert.ok(start > 0 && end > start, "stepTitle not found in App.svelte");
const stepTitle = new Function(src.slice(start, end + 4) + "\n  return stepTitle;")();

const at = (name, args) => stepTitle({ name, args: JSON.stringify(args) });
const cases = [
  ["read_file", { path: "C:\\w\\src\\App.svelte", start_line: 5900 }, "Read App.svelte from line 5900"],
  ["read_file", { filePath: "src/a.ts" }, "Read a.ts"],
  ["write_file", { path: "src/x.ts" }, "Edited x.ts"],
  ["run_shell", { command: "grep -n foo\nsecond line" }, "Ran grep -n foo"],
  ["search_files", { pattern: "^\\s*\\[" }, "Searched for ^\\s*\\["],
  ["webrain_serp", { q: "flights", engine: "auto" }, "Searched the web for “flights”"],
  ["webrain_serp", { q: "flights", engine: "brave" }, "Searched brave for “flights”"],
  ["webrain_navigate", { url: "https://www.example.com/a" }, "Opened example.com"],
  ["webrain_batch", { op: "markdown", urls: ["a", "b", "c"] }, "Ran 3 urls (markdown)"],
  ["webrain_scrape", { url: "https://api.github.com/x" }, "Read api.github.com"],
  ["skill", { skill: "webrain" }, "Loaded the webrain skill"],
  ["terminal", { op: "start" }, "Started a terminal"],
  ["terminal", { op: "send", input: "tasklist | findstr chrome" }, "Ran tasklist | findstr chrome"],
  ["fancy_tool", {}, "fancy tool"],
];

for (const [name, args, want] of cases) {
  assert.equal(at(name, args), want, `${name} ${JSON.stringify(args)}`);
}

// Degrades instead of printing a placeholder.
assert.equal(stepTitle({ name: "read_file", args: "" }), "Read a file", "no args");
assert.equal(stepTitle({ name: "read_file", args: "{not json" }), "Read a file", "bad json");
assert.equal(stepTitle({}), "tool", "no name");
for (const [name, args] of cases) {
  const got = at(name, args);
  assert.doesNotMatch(got, /undefined|null|NaN|\[object/, `${name} leaked a placeholder: ${got}`);
}

// A long argument is cut, and says so.
const long = at("webrain_serp", { q: "x".repeat(200) });
assert.ok(long.endsWith("”") && long.includes("…"), `long query not cut: ${long}`);

console.log(`step titles: ${cases.length + 6} checks passed`);
