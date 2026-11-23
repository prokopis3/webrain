// The composer's settable keys, checked without a browser. `keyName` is the one
// piece of real logic: the keydown handler compares against a canonical string,
// so a modifier order that differs from what the setting stored would silently
// stop matching — the shortcut would look set and never fire.
//
//   node check-keys.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");
const start = src.indexOf("  function keyName(e) {");
const end = src.indexOf("\n  }", start);
assert.ok(start > 0 && end > start, "keyName not found in App.svelte");
const keyName = new Function(src.slice(start, end + 4) + "\n  return keyName;")();

const ev = (key, mods = {}) => ({
  key,
  ctrlKey: !!mods.ctrl,
  metaKey: !!mods.meta,
  altKey: !!mods.alt,
  shiftKey: !!mods.shift,
});

// The three defaults the Settings tab offers, typed the way a keyboard reports
// them and compared the way the handler compares them.
assert.equal(keyName(ev("Tab")), "Tab");
assert.equal(keyName(ev("Enter", { ctrl: true })), "Ctrl+Enter");
assert.equal(keyName(ev("Enter", { alt: true })), "Alt+Enter");
// Order is fixed regardless of the order the user pressed them in.
assert.equal(keyName(ev("K", { ctrl: true, shift: true })), "Ctrl+Shift+K");
assert.equal(keyName(ev("K", { shift: true, ctrl: true })), "Ctrl+Shift+K");
// A bare Enter is not Ctrl+Enter: that difference is what keeps send working.
assert.notEqual(keyName(ev("Enter")), keyName(ev("Enter", { ctrl: true })));
// Space has a name, and a one-letter key is upper-cased so the stored value is
// the same letter on every keyboard layout that reports it.
assert.equal(keyName(ev(" ")), "Space");
assert.equal(keyName(ev("k")), "K");
assert.equal(keyName(ev("k", { alt: true, ctrl: true, shift: true, meta: true })), "Ctrl+Meta+Alt+Shift+K");

// Every action in the table is read by the composer, or it is a button that
// stores a key nothing listens for.
for (const id of ["complete", "stopSend", "queue"]) {
  assert.ok(src.includes(`keys.${id}`), `keys.${id} is never compared`);
}
// Tab completes; Enter runs. The Enter guard comes first and the completion key
// is what falls through to the field, so a Tab can never execute a command.
const enterGuard = src.indexOf('if (e.key === "Enter") {');
const tabGuard = src.indexOf("if (keyName(e) === keys.complete) {");
const pickCall = src.indexOf("return pick(menuRows[palette.index]);");
const fillCall = src.indexOf("return fillIn(menuRows[palette.index]);");
assert.ok(enterGuard > 0 && pickCall > enterGuard, "the palette has no Enter→pick path");
assert.ok(tabGuard > enterGuard, "the completion key is not checked after Enter");
assert.ok(fillCall > tabGuard, "Tab does not fill the name");
assert.ok(src.includes('localStorage.setItem("webrain-chat-keys"'), "the keys are not persisted");

// The running controls belong to the conversation that owns the turn. `current`
// moves the moment another thread is opened, so deriving the owner from it drew
// the Stop button — and the rail's dot — onto whichever chat you looked at, and
// every chat read as running while one of them was.
assert.ok(
  src.includes('const runningConvo = $derived(runningIn || (busy ? busyConvo : ""));'),
  "runningConvo is derived from `current`, which moves when you open a thread",
);
const busySets = (src.match(/busy = true;/g) || []).length;
const owners = (src.match(/busyConvo = current;/g) || []).length;
assert.ok(busySets > 0, "no `busy = true` found");
assert.equal(owners, busySets, "a `busy = true` that does not record which conversation it belongs to");
assert.ok(src.includes("{#if busy && runningConvo === current}"), "the stop controls are not scoped to the owner");
assert.ok(src.includes("const mine = busy && runningConvo === current;"), "the keystroke path is not scoped to the owner");
assert.ok(
  src.includes("if (!busy || runningConvo !== current) return;"),
  "stopTurn would abort another conversation's turn",
);

console.log("keys: 17 assertions passed");
