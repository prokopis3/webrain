// logo-nav.js — bump the Mintlify navbar logo from h-7 (28px) to h-10 (40px).
// Loaded via deployment-level customScripts (online admin setting), not docs.json
// (the docs.json schema strips/rejects custom CSS+JS). Runs on every page load.
(function () {
  var s = document.createElement("style");
  s.textContent = ".nav-logo { height: 2.5rem !important; }";
  document.head.appendChild(s);
})();
