// logo-nav.js — bump the Mintlify navbar logo from h-7 (28px) to h-10 (40px).
// ponytail: Mintlify strips custom CSS from docs.json, so the customScript
// integration is the only sanctioned injection point; runs on every page load.
(function () {
  var s = document.createElement("style");
  s.textContent = ".nav-logo { height: 2.5rem !important; }";
  document.head.appendChild(s);
})();
