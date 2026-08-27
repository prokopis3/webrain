Open Code Review Viewer
/
webrain
/
bf6be8d3-162…
Session: bf6be8d3-162f-401e-81d8-ce7efff89380
CWD:
D:\Windows\Documents\Programming\Projects\Rust\webrain
Branch:
Mode:
full_scan
Model:
deepseek-v4-flash
Duration:
47m32s
Files:
51
Status:
legacy
Token Usage
2.42M
Prompt Tokens
782.07K
Completion Tokens
3.2M
Total Tokens
191
LLM Requests
1.76M
Cache Read
0
Cache Write
2
LLM Failures
File breakdown
51 files
File	Prompt	Completion	Cache Read	Cache Write	Total
webrain-mcp/src/tools.rs	281.23K	25.78K	206.46K	0	307.01K
docs/styles/global.css	211.41K	28.6K	181.38K	0	240.01K
webrain-core/src/engines.rs	141.29K	33.71K	89.34K	0	175K
webrain-core/src/backends/cdp.rs	135.56K	29.36K	53.89K	0	164.93K
webrain-core/src/serp.rs	112.83K	49.26K	73.22K	0	162.09K
webrain-core/src/vision.rs	117.57K	35.23K	99.46K	0	152.8K
webrain-core/Cargo.toml	119.19K	20.71K	109.7K	0	139.9K
webrain-core/src/video.rs	109.05K	24.34K	72.83K	0	133.38K
webrain-core/src/install.rs	72.91K	47.25K	36.35K	0	120.15K
webrain-mcp/src/lib.rs	67.74K	39.04K	36.99K	0	106.78K
.dockerignore	76.53K	19.27K	74.5K	0	95.79K
webrain-cli/src/main.rs	79.72K	15.82K	65.28K	0	95.55K
docs/styles/landing-anim.js	59.83K	33.36K	30.21K	0	93.19K
docs/.mintignore	76.84K	15.56K	72.96K	0	92.4K
webrain-core/src/login.rs	70.81K	17.44K	59.52K	0	88.25K
docker/Dockerfile	60.71K	11.53K	54.14K	0	72.24K
webrain-core/src/browser.rs	49.6K	20.2K	37.89K	0	69.8K
.github/workflows/changelog-enforce.yml	35.43K	29.14K	31.23K	0	64.57K
docker/docker-compose.yml	53.44K	8.8K	42.37K	0	62.24K
.github/workflows/release.yml	28.44K	26.93K	8.19K	0	55.38K
webrain-core/src/captcha.rs	39.41K	13.78K	31.23K	0	53.19K
webrain-core/src/vault.rs	34.48K	18.45K	21.63K	0	52.93K
scripts/port_blocklist.ps1	39.76K	10.8K	37.5K	0	50.56K
webrain-core/src/launch.rs	29.71K	18.21K	18.3K	0	47.92K
__scan_dedup_batch_4__	17.74K	24.25K	256	0	41.99K
webrain-mcp/Cargo.toml	31.8K	9.83K	28.67K	0	41.63K
.github/dependabot.yml	33.05K	4.91K	28.03K	0	37.96K
webrain-core/src/lib.rs	33.01K	4.83K	28.67K	0	37.85K
scripts/install.sh	24.15K	11.98K	16.13K	0	36.14K
.github/workflows/pr-lint.yml	16.07K	12.79K	12.67K	0	28.86K
docs/logo-nav.js	14.08K	14.04K	10.62K	0	28.12K
webrain-cli/Cargo.toml	17.69K	10.33K	15.87K	0	28.01K
Cargo.toml	19.29K	7.83K	16.51K	0	27.12K
scripts/install.ps1	12.71K	8.72K	5.76K	0	21.43K
skills/webrain/scripts/build-skill.sh	12.38K	7.29K	11.26K	0	19.67K
.github/workflows/ci.yml	12.7K	6.09K	10.24K	0	18.8K
__scan_dedup_batch_7__	4.2K	13.05K	256	0	17.25K
commitlint.config.js	11.45K	4.86K	6.4K	0	16.31K
__scan_project_summary__	11.52K	4.24K	0	0	15.76K
docs/images/landing/hero-terminal-veo.json	4.95K	8.98K	2.56K	0	13.93K
__scan_dedup_batch_6__	2.24K	8.56K	256	0	10.8K
__scan_dedup_batch_1__	2.67K	7.57K	0	0	10.24K
.gitignore	6.98K	2.33K	6.27K	0	9.31K
docs/docs.json	4.8K	3.72K	2.56K	0	8.52K
docs/images/landing/hero-terminal-veo-manual.json	5.31K	2.37K	2.56K	0	7.68K
__scan_dedup_batch_8__	1.88K	4.84K	256	0	6.73K
.github/FUNDING.yml	3.42K	2.92K	2.56K	0	6.34K
webrain-core/src/backends/mod.rs	5.41K	656	2.56K	0	6.07K
__scan_dedup_batch_3__	2.01K	1.68K	256	0	3.69K
__scan_dedup_batch_5__	1.39K	643	256	0	2.03K
__scan_dedup_batch_0__	1.36K	188	0	0	1.54K
Review Comments (181 findings)
Severity:
All: 181
Critical: 1
High: 36
Medium: 104
Low: 40
Category:
All: 181
Bug: 95
Security: 32
Performance: 15
Maintainability: 30
Style: 1
Other: 8
bug
medium
L128
Coverflow card order is effectively reversed. Because the delay is negative (`-7s * card`), each card starts further ahead in its 28s cycle, so the front windows are: card0 [0–6.2s], card3/dup [7–13.2s], card2 [14–20.2s], card1 [21–27.2s]. With `--card` assigned 0,1,2,3 in document order (docs/index.mdx), the viewer sees: card0 → dup-of-card0 → card2 → card1. That means the "scrape" demo plays twice back-to-back and the second/third demos appear swapped — the trailing duplicate that is supposed to make the loop seamless actually follows the original immediately. To get the intended 0 → 1 → 2 → dup(0) sequence, the markup should assign `--card` as 0, 3, 2, 1 (delays 0, -21s, -14s, -7s), or the delay formula should be adjusted accordingly.
Existing Code
.landing .wf-card3d { position: absolute; inset: 0; flex: none; backface-visibility: hidden; animation: wf-cover 28s cubic-bezier(0.32, 0.72, 0, 1) both infinite; animation-delay: calc(var(--card, 0) * -7s); }
bug
medium
L381
`outline: 0` removes the browser's default focus ring from the playground prompt, and no replacement focus style exists anywhere in this file (no `.try-input:focus-visible`, no `.try-prompt:focus-within`). Keyboard users tabbing into the input get no visible focus indicator, which is an accessibility regression. Add e.g. `.landing .try-input:focus-visible { outline: 2px solid var(--accent-2); outline-offset: 2px; }` or a `:focus-within` highlight on `.try-prompt`.
Existing Code
.landing .try-input { flex: 1; min-width: 0; background: none; border: 0; outline: 0; color: var(--text); font-family: 'Geist Mono', ui-monospace, monospace; font-size: 0.8rem; caret-color: var(--accent-2); }
bug
low
L130
Hover only pauses the card rotation (`.wf-card3d`), but the transcript streaming lives on the descendant `.wf-body > .wf-group` animations, which keep running. On hover the front card can freeze mid-cycle while its text continues typing/fading (or fades out entirely), and cards frozen in hidden positions keep re-streaming invisibly. Pause the group animations too, e.g. `.landing .wf-stage:hover .wf-body > .wf-group { animation-play-state: paused; }`.
Existing Code
.landing .wf-stage:hover .wf-card3d { animation-play-state: paused; }
other
low
L511
`.bar-fill` starts at `width: 0` and only the JS/anime layer grows it on scroll. If landing-anim.js fails to load (or runs after the CDN), the benchmark bars render as permanently empty tracks — the visual data is hidden, contradicting the file's stated 'content never hidden / no-JS safe' principle. Consider a static default width (e.g. `width: var(--fill, 0)`) so no-JS users still see the comparison, and let the JS override it.
Existing Code
.landing .bar-fill { height: 100%; width: 0; border-radius: 5px; position: relative; overflow: hidden; }
performance
low
L366
`will-change: transform, opacity` is applied permanently to every hero word span. Each `.w` is promoted to its own compositor layer for the entire page lifetime, even when the anime.js word-split/entrance never runs (no-JS) or under `prefers-reduced-motion: reduce` where these words are never animated. Prefer toggling `will-change` only while the animation is active (add/remove a class from landing-anim.js) or scope it under `prefers-reduced-motion: no-preference`.
Existing Code
.landing h1 .w { display: inline-block; will-change: transform, opacity; }
bug
medium
L22
The header comment states that scopes are enforced on PR titles, but `'scope-empty': [0]` disables the scope-required check (level 0 = rule disabled). As a result, a commit like `feat: add feature` with no scope passes validation, contradicting the documented intent. If scopes are meant to be mandatory, change to `'scope-empty': [2, 'never']`; otherwise update the comment to say scopes are optional but validated when present.
Existing Code
    'scope-empty': [0],
Suggested Change
    'scope-empty': [2, 'never'],
maintainability
low
L18-L20
The values `docs`, `build`, `ci`, `style`, `perf`, and `test` appear in both `type-enum` and `scope-enum`. This makes classification ambiguous (e.g., a commit `docs(docs): ...` is technically valid) and forces manual synchronization whenever either list changes. Consider deduplicating — keep the overlapping values in `type-enum` and remove them from `scope-enum` (or derive one list from the other) so the two lists stay in sync.
Existing Code
      // housekeeping
      'docs', 'build', 'ci', 'style', 'perf', 'dist',
      'deps', 'config', 'test', 'skill', 'script', 'release',
maintainability
low
L5-L6
This config only validates `type-enum` and `scope-enum`. It neither extends `@commitlint/config-conventional` nor defines rules such as `type-case`, `type-empty`, `subject-empty`, `scope-case`, or `header-max-length`. Consequently, malformed, empty, or overlong commit subjects (e.g., a 300-char PR title) are not rejected, weakening the stated PR-title validation. Consider adding `extends: ['@commitlint/config-conventional']` and/or explicit rules like `'header-max-length': [2, 'always', 100]` and `'subject-empty': [2, 'never']`.
Existing Code
  rules: {
    'type-enum': [2, 'always', [
bug
medium
L5-L7
`document.head` may be null if this script executes before the <head> element is parsed (the injection point/order of deployment-level customScripts is not fully controlled). If so, `appendChild` throws and the exception aborts the remainder of the script block, so the OS-tab IIFE below never runs. Guard the append target.
Existing Code
  var s = document.createElement("style");
  s.textContent = ".nav-logo { height: 2.5rem !important; }";
  document.head.appendChild(s);
Suggested Change
  var s = document.createElement("style");
  s.textContent = ".nav-logo { height: 2.5rem !important; }";
  (document.head || document.documentElement).appendChild(s);
bug
medium
L37-L39
`userClicked` is only set by a 'click' event. Keyboard-driven tab selection (ARIA tabs pattern: arrow keys / Enter) or programmatic state changes never set it, so the polling loop keeps re-applying the detected OS over a non-click user choice. Also, this capture-phase listener is never removed and stays active for the page's lifetime. Consider also listening for keydown on the tab container and removing listeners once the selection is settled.
Existing Code
  document.addEventListener("click", function (e) {
    if (e.target && e.target.closest && e.target.closest(".install-os")) userClicked = true;
  }, true);
performance
medium
L40-L45
The loop always runs the full 50 iterations (~7.5 s) even after the install block has been found and applied, and the 'load' listener triggers one more redundant apply. Neither the loop nor the listeners are torn down afterwards, leaving unnecessary DOM work (re-writing textContent/aria-selected every 150 ms, which can even disrupt a user mid-copy of the command) that lingers across Mintlify SPA navigations. Early-exit once `blk` exists and the data-cmd attribute is populated, and remove the 'load' listener after it fires.
Existing Code
  var n = 0;
  (function loop() {
    if (!userClicked) apply(os);
    if (++n < 50) setTimeout(loop, 150); // ~7.5s ceiling
  })();
  window.addEventListener("load", function () { if (!userClicked) apply(os); });
style
low
L35-L36
The file uses `var` throughout, which violates the project's strict let/const rule. Additionally, the shared mutable closure state (`n`, `userClicked`) is only ever mutated inside the anonymous loop/listener callbacks, which compounds the lifecycle/cleanup concerns above (harder to reset or clear when the page navigates).
Existing Code
  var os = detectOS();
  var userClicked = false;
bug
high
L385-L387
The IO callback re-reveals elements that were already revealed. IntersectionObserver fires an initial callback with the current intersection state for every target right after observe(), so any in-viewport element already revealed by the immediate branch above (revealed.add + reveal) will immediately be revealed a second time. reveal() always animates opacity from [0,1], so every in-viewport section visibly flashes (hide -> show) on first load; scrolling away and back also re-triggers the animation. Guard with a revealed.has check and unobserve before calling reveal.
Existing Code
        if (!en.isIntersecting) return;
        revealed.add(en.target);
        reveal(en.target, Number(en.target.getAttribute('data-i') || 0));
Suggested Change
        if (!en.isIntersecting) return;
        if (revealed.has(en.target)) { scrollIo.unobserve(en.target); return; }
        revealed.add(en.target);
        reveal(en.target, Number(en.target.getAttribute('data-i') || 0));
bug
medium
L920-L923
runPlayground can be invoked repeatedly (window.__webrain.run from preset clicks) while a previous run is still streaming, but it never cancels the previous run's pending work: the old .finished.then continuation still calls streamLines(linesEl, oldDemo.lines, 0), appending the previous demo's lines into the freshly cleared terminal, and the old showCard setTimeout fires after the new card is shown and overwrites it with the previous demo's card. Introduce a run generation token (increment at each run and capture it in the closures, bailing when stale) or keep and clear references to the pending timeout/animation.
Existing Code
    }).finished.then(function () {
      streamLines(linesEl, demo.lines, 0);
      setTimeout(function () { showCard(cardEl, demo.card); }, demo.lines.length * 480 + 320);
    });
bug
medium
L128-L134
The poll only terminates when macOS is active. If the user clicks another OS tab before hydration settles (the interval can live up to 6 attempts ≈ 11s), ensureInstallDefault forcibly re-selects macOS, directly contradicting the stated intent ('so it never fights a user click on another OS later'). Stop the poll as soon as a user interacts with the tabs (e.g., a one-time click listener that clears the interval), or stop after the first pass where the active tab matches the settled React state rather than forcing the macOS default.
Existing Code
  var installer = setInterval(function () {
    var mac = document.querySelector('.landing .install-os[data-os="macos"]');
    var active = document.querySelector('.landing .install-os.active');
    if (mac && active === mac) { clearInterval(installer); return; }
    ensureInstallDefault();
    if (++tries >= 6) clearInterval(installer);
  }, 1800);
bug
medium
L396-L402
The no-IntersectionObserver fallback never sets data-anim/data-i and never pre-hides the targets before animating. reveal() reads data-anim (falls back to 'rise' for every element, losing the per-group fade/cell/slideL/check animations) and animates opacity from [0,1] without a prior hidden state, so the fully-visible page flashes to opacity 0 and back on load. The `i` passed here is also the global index across all groups, not the per-group index used elsewhere, producing incorrect stagger delays. Reuse the same tagging + pre-hide logic as setupScrollReveals, or simply leave everything visible without animating in this fallback.
Existing Code
    var allTargets = [];
    scrollGroups.forEach(function (g) {
      Array.prototype.forEach.call(document.querySelectorAll(g.sel), function (el) {
        allTargets.push(el);
      });
    });
    allTargets.forEach(function (el, i) { reveal(el, i); });
maintainability
medium
L12-L13
The entire module uses `var`, but the project rules strictly prohibit `var` in favor of let/const. This also matters for correctness clarity here (e.g., `tries`, `reapplied`, `loopRunner`/`heroWebRun` are mutated from closures). Convert all declarations to let/const.
Existing Code
  var REDUCE = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var HAS = !!window.anime;
performance
low
L83-L86
In the steady state (entrance played and .w spans present), the guard condition `h1.querySelectorAll('.w').length === 0` is false, so `reapplied` never increments and the interval never clears — it keeps querying the DOM every 1300ms forever. Clear the interval once the steady state is observed (e.g., also clear when `played && h1.querySelectorAll('.w').length > 0`).
Existing Code
      var guard = setInterval(function () {
        var h1 = document.querySelector('.landing .hero h1');
        if (!h1 || reapplied >= 3) { clearInterval(guard); return; }
        if (h1.querySelectorAll('.w').length === 0 && h1.textContent.indexOf('Browser') !== -1) {
security
high
L17-L19
The downloaded `webrain.exe` is written to disk and later executed (per the script's own next-step instructions) with no checksum or Authenticode verification. A compromised release, a tampered mirror, or a MITM in the download chain would silently deliver arbitrary code running with the user's privileges. Pin the expected SHA-256 hash for the release asset and compare it before install, or at minimum verify the Authenticode signature of the downloaded binary before use.
Existing Code
$url = "https://github.com/$Repo/releases/latest/download/webrain-windows.exe"
Write-Host "webrain: downloading $url"
Invoke-WebRequest $url -OutFile $exe -UseBasicParsing
security
medium
L21-L23
The PATH check uses a substring wildcard (`-notlike "*$InstallDir*"`), so an existing entry such as `...\Programs\webrain-old` or `...\webrain2` satisfies the match and the real install dir is never appended — the `webrain` command then stays unavailable after a successful "install". Additionally, if the user Path is empty/null, `"$userPath;$InstallDir"` starts with `;`, producing an empty PATH segment that some shells interpret as the current directory (a potential binary-hijack vector). Prefer splitting the user Path on `;`, comparing entries exactly (case-insensitive, trailing `\` normalized), and joining only non-empty entries.
Existing Code
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$InstallDir", 'User')
bug
medium
L13
If `LOCALAPPDATA` is not set (e.g., SYSTEM context, some service accounts, or unusual environments), `Join-Path` produces a relative path and the binary is installed under the current working directory instead of the intended `%LOCALAPPDATA%\Programs\webrain`, and the wrong path is added to PATH. Add a fallback base directory, e.g.: `$base = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { Join-Path $env:USERPROFILE 'AppData\Local' }` before building `$InstallDir`.
Existing Code
$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\webrain'
bug
medium
L16-L19
The installer writes directly to the final `$exe` path with no temporary file, no check for an already-running `webrain.exe`, and no rollback. If an instance is running (e.g., an MCP server), `-OutFile` fails on the file lock, and if the download is interrupted the partially written file is left in place, silently corrupting a previously installed version while the script reports success only in the happy path. Recommend downloading to a temp file, validating it, then `Move-Item` (after optionally stopping a running instance) so the final path is only ever replaced atomically.
Existing Code
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$url = "https://github.com/$Repo/releases/latest/download/webrain-windows.exe"
Write-Host "webrain: downloading $url"
Invoke-WebRequest $url -OutFile $exe -UseBasicParsing
bug
medium
L15-L16
The validation regex `^[a-z0-9.-]+\.[a-z]{2,}$` accepts malformed hostnames: leading/trailing hyphens in labels (`-foo.com`, `foo-.com`), consecutive dots (`foo..com`), a leading dot (`.example.com`), and labels longer than 63 chars. These invalid entries are written straight into `tracker_domains.txt`, which is consumed via `include_str!` + exact host matching in `cdp.rs` — such entries will never match a real host and silently bloat the 3500-slot list. Validate per RFC 1123 instead (split on '.', require ≥2 labels, each label `^[a-z0-9]([a-z0-9-]*[a-z0-9])?$`, length ≤63).
Existing Code
} | Where-Object {
    $_ -match '^[a-z0-9.-]+\.[a-z]{2,}$'
Suggested Change
} | Where-Object {
    $labels = $_.Split('.')
    $labels.Count -ge 2 -and $labels[-1].Length -ge 2 -and
    ($labels | Where-Object { $_ -notmatch '^[a-z0-9]([a-z0-9-]*[a-z0-9])?$' -or $_.Length -gt 63 }).Count -eq 0
bug
high
L18-L19
There is no minimum parsed-count guard before `Set-Content` overwrites the production file. If the upstream URL returns an HTML error page, a 404 body, a format change, or a transiently empty response that doesn't raise an exception, `$domains` can be empty or severely truncated and `tracker_domains.txt` is silently clobbered — destroying the previous good blocklist with no recovery. Add a sanity check (e.g., throw if fewer than ~1000 domains parsed) before writing.
Existing Code
$take = $domains | Select-Object -First 3500
Set-Content -Path $out -Value $take -Encoding utf8
Suggested Change
$take = $domains | Select-Object -First 3500
if ($take.Count -lt 1000) { throw "Parsed only $($take.Count) domains; aborting to avoid truncating tracker_domains.txt" }
Set-Content -Path $out -Value $take -Encoding utf8
bug
medium
L19
`Set-Content -Encoding utf8` behavior differs across PowerShell versions: Windows PowerShell 5.1 writes a UTF-8 BOM, PowerShell 7+ writes BOM-less. `tracker_domains.txt` is embedded via Rust `include_str!` and split on lines (cdp.rs), which does not strip a BOM — under 5.1 the first domain would carry a leading U+FEFF and never match a real host in exact-match lookups. Write an explicit BOM-less UTF-8 encoding for deterministic output across hosts.
Existing Code
Set-Content -Path $out -Value $take -Encoding utf8
Suggested Change
[System.IO.File]::WriteAllLines($out, [string[]]$take, (New-Object System.Text.UTF8Encoding($false)))
bug
medium
L7
The single `Invoke-WebRequest` has no retry/backoff, so any transient GitHub/proxy failure aborts the port even though `$ErrorActionPreference='Stop'` already leaves the old file intact (the abort itself is safe, but unattended runs get no recovery). Additionally, on Windows PowerShell 5.1 with older .NET, the default `ServicePointManager` security protocol may not include TLS 1.2, causing `Invoke-WebRequest` against GitHub to fail with "Could not create SSL/TLS secure channel". Add TLS 1.2 negotiation plus a small retry loop with backoff.
Existing Code
$r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 60
Suggested Change
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$r = $null
for ($i = 0; $i -lt 3 -and -not $r; $i++) {
    try { $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 60 }
    catch { if ($i -eq 2) { throw }; Start-Sleep -Seconds (5 * ($i + 1)) }
}
maintainability
low
L18
`Sort-Object -Unique` then `Select-Object -First 3500` truncates the list alphabetically, so the tail of the alphabet is always dropped and early-alphabet domains crowd out others. Since the target is a fixed ~3500-slot list with a hard cap, this deterministic but arbitrary bias means tracker coverage depends purely on domain name ordering. Consider documenting the selection policy or choosing a less biased sample (e.g., stable shuffle) so the ported list is representative.
Existing Code
$take = $domains | Select-Object -First 3500
security
high
L77-L83
The module comment promises "POST to 2captcha in.php", but the code serializes all form params — including the API key and (when configured) the proxy credentials — into the GET URL query string and fetches it with `serp_http_get`. Query strings are routinely captured by access logs, reverse proxies/load balancers, and request tracing, so the secrets are far more exposed than a form-encoded POST body would be; long proxy credentials can also blow past URL length limits and get truncated. Send the params as an `application/x-www-form-urlencoded` POST body (e.g. a `serp_http_post` helper wrapping ureq `.send_form`) so the secrets stay out of the URL.
Existing Code
    let submit = format!(
        "{IN_URL}?{}",
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish()
    );
    let (_, body) = crate::engines::serp_http_get(&submit, None)?;
Suggested Change
    // POST the form body instead of placing the API key / proxy credentials in the URL.
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish();
    let (_, body) = crate::engines::serp_http_post(IN_URL, &body, None)?;
bug
medium
L93-L94
Each res.php poll uses `?` on the transport call, so a single transient network error aborts the entire solve — while the already-submitted (and possibly charged) 2captcha task is still pending server-side. The HTTP status is also discarded (`(_, body)`), so a 5xx/HTML response is treated as a fatal result instead of a retryable blip. Treat transport errors and non-200 responses as transient (keep polling, e.g. with a consecutive-failure cap), and only fail the solve on explicit 2captcha error bodies such as `ERROR_CAPTCHA_UNSOLVABLE`.
Existing Code
        let poll = format!("{RES_URL}?key={api_key}&action=get&id={id}");
        let (_, body) = crate::engines::serp_http_get(&poll, None)?;
Suggested Change
        let poll = format!("{RES_URL}?key={api_key}&action=get&id={id}");
        let body = match crate::engines::serp_http_get(&poll, None) {
            Ok((status, body)) if status >= 500 => continue, // transient server error
            Ok((_, body)) => body.trim().to_string(),
            Err(_) => continue, // transient transport error; task may still be pending
        };
bug
medium
L39-L41
Proxy handling silently degrades in several ways, which can make the returned token unusable for the real egress and waste paid solves: (1) an unparseable proxy URL is silently ignored (`if let Ok(u) = ...`) instead of failing loudly; (2) `u.username()`/`u.password()` return percent-encoded values that are forwarded un-decoded, mangling credentials containing `@`, `:`, spaces, etc.; (3) every `socks*` scheme — including socks4/socks4a — is mislabeled as SOCKS5; (4) a proxy URL without an explicit port yields a `proxy` value with no port, which 2captcha cannot use. Validate the scheme/port and propagate an error on invalid input, and percent-decode the userinfo before sending.
Existing Code
    if let Some(raw) = proxy {
        if let Ok(u) = url::Url::parse(raw) {
            if let Some(host) = u.host_str() {
Suggested Change
    if let Some(raw) = proxy {
        let u = url::Url::parse(raw)
            .with_context(|| format!("invalid captcha proxy URL: {raw}"))?;
        if let Some(host) = u.host_str() {
security
high
L304-L312
Security: unlike Chrome (which is explicitly bound to 127.0.0.1 via `--remote-debugging-address`), this binds the raw, unauthenticated CDP server to 0.0.0.0 — any peer on the network that can reach the host gets full browser control with no authentication. The `--advertise-host 127.0.0.1` only changes the URL advertised to clients, not the listen address. Bind `--host 127.0.0.1` instead.
Existing Code
    let args = vec![
        "serve".to_string(),
        "--host".to_string(),
        "0.0.0.0".to_string(),
        "--port".to_string(),
        port.to_string(),
        "--advertise-host".to_string(),
        "127.0.0.1".to_string(),
    ];
security
high
L336-L343
Security: same as lightpanda — `--host 0.0.0.0` exposes unauthenticated CDP (with `--stealth` masking enabled) on all network interfaces, whereas the Chrome launch path deliberately binds remote debugging to 127.0.0.1. Bind to 127.0.0.1.
Existing Code
    let args = vec![
        "serve".to_string(),
        "--host".to_string(),
        "0.0.0.0".to_string(),
        "--port".to_string(),
        port.to_string(),
        "--stealth".to_string(),
    ];
bug
medium
L184-L186
Check-then-act race / stale listener: the port is probed here and readiness is later verified only by a plain TCP connect in `spawn_and_wait`. Between the check and the actual bind, a concurrent launch (or any unrelated process) can grab the port, so two instances can collide, or the wait loop can report success against a foreign listener that is not the spawned engine's CDP. Consider binding the port exclusively yourself (and passing the fd/port to Chrome), or at least verifying the endpoint actually serves the expected CDP (e.g. GET /json/version) and that the child is still alive before declaring readiness.
Existing Code
    if port_open(port) {
        anyhow::bail!("port {port} already has a CDP endpoint — another Chrome is running there");
    }
bug
medium
L160-L165
The wait loop never calls `child.try_wait()`, so a child that exits immediately (bad binary, incompatible flags, crash during startup) is only noticed after the full 20s timeout, and the caller gets a misleading "did not open CDP within 20s" error instead of the real exit status. Add a `try_wait()` check each iteration and bail with the child's exit status when it has died.
Existing Code
    while Instant::now() < deadline {
        if port_open(port) {
            return Ok(launched);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
bug
medium
L129-L133
`kill()` sends the signal but never reaps the child — `std::process::Child` does not `wait()` in its own `Drop`, so on Unix every killed Chrome becomes a zombie until the parent exits. A long-lived parent (daemon/sidecar) will accumulate zombies despite the "kills Chrome on drop (no orphans)" intent. Call `self.child.wait()` after `kill()` (or otherwise mark the child as reaped) to collect the exit status.
Existing Code
impl Drop for Launched {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
security
medium
L187-L188
`service`/`profile` are user-supplied strings joined directly into the Chrome `--user-data-dir` with no validation: a profile/service name containing `/` or `..` escapes the intended profiles root and writes profile data to an arbitrary path (same pattern in `launch_chrome_pipe`). Validate the components (reject path separators and `..`, or map them through a sanitizer/hash) before building the path.
Existing Code
    let profile_dir = profiles_dir().join(service).join(profile);
    std::fs::create_dir_all(&profile_dir)?;
bug
medium
L44-L47
Ordering contradicts the documented intent: the header comment says "real Chrome FIRST ... then CfT as fallback", but on macOS/Linux this `find_cft_chrome()` check runs *before* the real Chrome/Edge install paths (macOS block) and the PATH lookup (google-chrome etc.), so CfT/Chromium is preferred over real Chrome on those platforms — undermining the stated anti-fingerprinting rationale. Move this block after the macOS install-path and PATH checks (on Windows it is already correctly placed after the real Chrome candidates).
Existing Code
    // CfT fallback only when no real Chrome/Edge is installed.
    if let Some(p) = crate::install::find_cft_chrome() {
        return p;
    }
maintainability
medium
L285-L286
Generic trait default is coupled to a concrete backend: `snapshot` reaches into `crate::backends::cdp::ELEMENTS_JS`/`LINKS_JS` (and the serde shape those scripts return), so the supposedly backend-agnostic `BrowserBackend` trait won't compile if the `cdp` module is feature-gated or renamed, and every non-CDP backend is silently bound to CDP's extraction contract. Consider hoisting these JS snippets into `browser.rs` as shared consts, or exposing them as trait/associated constants so backends can override or reuse them without depending on a specific backend module.
Existing Code
        let elements: Vec<InteractiveElement> =
            serde_json::from_value(self.evaluate(crate::backends::cdp::ELEMENTS_JS).await?)
bug
medium
L267-L272
Extraction failures are silently flattened: `unwrap_or("")` / `unwrap_or_default()` turn JS errors surfaced as null/undefined, non-string results, or schema drift (e.g., a changed `ELEMENTS_JS`/`LINKS_JS` return shape) into empty url/title/text/elements/links. Callers can't distinguish a genuinely empty page from a broken backend — and worse, an empty `elements` feeds `detect_crippled` (0 < 5), mislabeling a healthy page whose extraction failed as a bot-limited "crippled" shell. Consider deserializing with error context (e.g., `serde_json::from_value(...).map_err(...)`) or returning the raw `Value` so failures are observable.
Existing Code
        let url = self
            .evaluate("location.href")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();
bug
medium
L67-L68
`detect_chrome_error` returns a code on the FIRST `ERR_`/`DNS_` substring found anywhere in title+visible text, without confirming the page is actually a Chrome interstitial. Ordinary pages that merely mention such codes (support articles, log dumps, network documentation) are classified as dead Chrome error pages and skipped by the LLM. Require a corroborating signature — e.g., only return the code when a known interstitial phrase from `PHRASES` is also present, or when the code appears in the `<title>` rather than anywhere in body text.
Existing Code
    for marker in ["ERR_", "DNS_"] {
        if let Some(i) = hay.find(marker) {
bug
low
L99-L101
`detect_crippled` is a bare count heuristic: any non-challenge page with < 5 interactive elements (legit login forms, landing pages, confirmation dialogs, or pages whose extraction silently failed) is flagged as a bot-limited shell, and the field is exposed to the LLM as a hint that the page is useless. The ponytail comment acknowledges the noise but gives callers no way to opt out or tune it. Combine signals (element count + page text length / link count / whether elements look like empty shells) or scope the threshold per-site, and consider not flagging when extraction itself failed.
Existing Code
pub fn detect_crippled(challenge: &Option<String>, element_count: usize) -> bool {
    challenge.is_none() && element_count < 5
}
bug
medium
L148-L149
`location.reload(true)` is non-standard: per the HTML spec `location.reload()` takes no arguments and modern browsers ignore the boolean, so this default fallback performs an ordinary, possibly cache-served reload — it cannot deliver the documented cache-bypassing "hard reload". The reload-triggered navigation can also destroy the JS context before this `evaluate` resolves, producing a spurious error. Either document this default as best-effort (real hard reloads belong in CDP backends via `Page.reload ignoreCache:true`), or drop the misleading boolean argument.
Existing Code
    async fn reload_hard(&self) -> anyhow::Result<()> {
        self.evaluate("location.reload(true); true")
bug
low
L46-L47
Markers like `forbidden`, `access denied`, and `checking your browser` are matched against the entire lowercased title+visible text, so legitimate pages that merely discuss these phrases (help articles, error-code docs, forums) get classified as `blocked`/`cloudflare_challenge`. Since `challenge` gates whether the LLM keeps scraping, consider restricting markers to title-only or requiring 2+ corroborating markers before declaring a challenge.
Existing Code
        ("blocked", "access denied"),
        ("blocked", "forbidden"),
bug
high
L55-L56
Sequential placeholder replacement corrupts credentials: `replace("PASS", ...)` runs after the username has already been substituted, so if the username contains the literal substring "PASS" (e.g. `myPASSword`), that substring gets rewritten to the JSON-escaped password and the submitted username is mangled. Either order is unsafe (reversing it would corrupt a password containing "USER"). Replace both placeholders in a single pass, or use sentinel values that can't collide with user data (e.g. two-phase replacement via control-character sentinels) so no substitution ever runs over the other's output.
Existing Code
    TPL.replace("USER", &jstr(user))
        .replace("PASS", &jstr(pass))
Suggested Change
    // Two-phase replacement: swap in sentinels first so the user/pass values can't
    // collide with the remaining placeholder (a username containing "PASS" would
    // otherwise be rewritten to the password value).
    TPL.replace("USER", "\u{1}USER\u{1}")
        .replace("PASS", "\u{1}PASS\u{1}")
        .replace("\u{1}USER\u{1}", &jstr(user))
        .replace("\u{1}PASS\u{1}", &jstr(pass))
bug
medium
L146
The captcha-token wait result is discarded even though the surrounding comment states "submitting with an EMPTY token is a 403". If the widget never populates within 30s (user didn't click, click claim failed, etc.), `wait_turnstile_token` returns false and the form is still submitted with an empty token, producing a guaranteed 403 that gets misreported as a credentials problem. Check the returned bool and return early with a `waiting_for_human` result instead of submitting blindly. (Same pattern applies to the discarded `_cleared` from `wait_out_challenge(90)` — if the anti-bot interstitial never clears, a form fill/submit is attempted on a page that has no login form.)
Existing Code
        let _tok = backend.wait_turnstile_token(30).await;
Suggested Change
        if !backend.wait_turnstile_token(30).await {
            return Ok(json!({
                "logged_in": false,
                "waiting_for_human": true,
                "challenge": "captcha",
                "message": "captcha token never populated — solve the widget in the headed browser, then call login again",
                "submitted": submitted,
            }));
        }
maintainability
medium
L89-L92
`gate_up`/`captcha_up` collapse every `evaluate()` failure into `false`, so a CDP error during the poll loop (e.g. "Execution context was destroyed" right after submit navigation) is indistinguishable from "no challenge present". The loop then burns the full 15s window and returns the misleading "no session cookie after 15s — check creds or log in manually" message, or silently misses a real 2FA/captcha gate. Make these return `anyhow::Result<bool>` and propagate/record evaluate errors so a failed probe is surfaced instead of treated as a negative.
Existing Code
    b.evaluate(twofa_js())
        .await
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(false)
other
low
L176-L180
TOTP auto-fill errors are fully swallowed. `vault::fill_js` returns `{ok:false, reason:'no-sel'}` when the OTP field isn't present on the page, and `totp_code(seed)` can fail too, but both the `if let Ok` guard and `let _ =` discard everything — the API still reports only the generic "2FA/approval gate" message, hiding a broken TOTP auto-fill (e.g. wrong seed, or a push-approval gate with no code field). Capture the evaluate result and include a `totp_filled: true/false` field (and/or the failure reason) in the returned JSON so the caller can distinguish "code entered" from "code injection failed".
Existing Code
                if let Ok(code) = vault::totp_code(seed) {
                    let _ = backend
                        .evaluate(&vault::fill_js(otp_selector(), &code))
                        .await;
                }
bug
low
L85
The `contains("session")` catch-all partially re-opens the false-positive bug the regression test below guards against. The exact-name list deliberately excludes login-page-only cookies (datr/dpr/mid/ig_did) that are present while logged OUT, but any site setting a non-auth cookie whose name contains "session" on the login page (e.g. `session_referrer`, `session_start`, `session_language`) would make `has_session()` report `logged_in: true` before the user ever submits. Consider restricting the substring match (e.g. require the value to look like a real session id) or keep it to well-known session cookie suffixes.
Existing Code
        .any(|n| SESSION_COOKIES.contains(&n) || n.to_ascii_lowercase().contains("session")))
bug
high
L644-L650
send_cmd_with holds the single connection Mutex across the entire response-wait loop with no timeout. If the browser never answers a command (renderer paused on a dialog, tab crash, half-open connection), `conn.read.recv()` never yields the matching id and EVERY subsequent command on EVERY tab blocks forever while holding the global lock. dispatch_click works around this with 2s timeouts, but navigate/evaluate/screenshot/cookies etc. have no equivalent protection. Wrap the wait loop in `tokio::time::timeout` (e.g. ~30s) and return an error so callers can fail fast / reconnect instead of hanging the whole backend.
Existing Code
        let text = serde_json::to_string(&msg)?;
        conn.write.send(text).await?;

        loop {
            let Some(text) = conn.read.recv().await else {
                anyhow::bail!("CDP connection closed");
            };
bug
high
L695-L698
Lock-order inversion: `register_tab` acquires `tabs` then `active`, while `active_session`, `list_tabs`, and `close_tab` acquire `active` then `tabs` (in close_tab the `active` guard is held while locking `tabs` again to pick the next tab). Two concurrent tasks (e.g. open_tab/ensure_page_attached racing close_tab) can deadlock permanently. Use a consistent order everywhere (active→tabs) or a single combined lock.
Existing Code
    async fn register_tab(&self, id: String, tab: Tab) {
        self.tabs.lock().await.insert(id.clone(), tab);
        *self.active.lock().await = id;
        *self.fp.lock().await = None;
Suggested Change
    async fn register_tab(&self, id: String, tab: Tab) {
        let mut active = self.active.lock().await;
        self.tabs.lock().await.insert(id.clone(), tab);
        *active = id;
        *self.fp.lock().await = None;
bug
high
L2601-L2604
`bid` here comes from the AX node's `backendDOMNodeId` (a browser-global id), but it is passed in the `nodeId` field of `DOM.getContentQuads`, which expects a document-scoped DOM `nodeId` (backend ids require the separate `backendNodeId` parameter). The call therefore fails (or targets the wrong node), the `.ok()?` swallows it, and `consent_button` always returns None — consent dialogs are never clicked. Pass `{"backendNodeId": bid}` instead.
Existing Code
            let quads = self
                .send_cmd("DOM.getContentQuads", json!({"nodeId": bid}))
                .await
                .ok()?;
Suggested Change
            let quads = self
                .send_cmd("DOM.getContentQuads", json!({"backendNodeId": bid}))
                .await
                .ok()?;
bug
high
L1628-L1629
ELEMENTS_JS falls back to a bare tag name (e.g. "button") when an element has neither id nor class, and to only the first class token otherwise. `DOM.querySelector` with such a selector resolves to the FIRST matching node in the document, not the element at `index`. `backend_click_center` is partially masked by the `click_landed` gate, but `resolve_node_id` (used by `set_file_inputs`) has no such gate: an id/class-less `<input type=file>` can resolve to a different (or non-file) input, silently setting files on the wrong node or erroring. Also `'#' + el.id` is unescaped, so ids containing `.`, `:`, etc. break querySelector. Generate a stable index-aware path (e.g. nth-of-type chain) or pass the index into the query.
Existing Code
    let elements: Value = b.eval_js(ELEMENTS_JS).await.ok()?;
    let selector = elements.as_array()?.get(index)?.get("selector")?.as_str()?;
bug
medium
L2173
The Enter JS fallback does not actually submit the form: dispatching a synthetic `Event('submit')` only fires onsubmit handlers — it does NOT run the browser's native form submission (no navigation/action), contradicting the comment above that claims "Enter keeps the JS path which dispatches form.submit()". On engines without `Input.dispatchKeyEvent` (lightpanda), pressing Enter will never submit. Use `el.form.submit()` (or `HTMLFormElement.prototype.requestSubmit.call(el.form)` to also fire the submit event with validation).
Existing Code
                if (el.form && k === 'Enter') {{ el.form.dispatchEvent(new Event('submit', {{ bubbles: true, cancelable: true }})); return 'form-submitted'; }}
Suggested Change
                if (el.form && k === 'Enter') {{ el.form.submit(); return 'form-submitted'; }}
bug
high
L1235-L1237
CLAIM_JS ends with `return JSON.stringify({...})`, so `eval_js` (returnByValue) yields a JSON *string*, and `res.as_object()` is always None. The claimed branch — including the trusted cross-origin CDP click on the captcha iframe — is dead code; only the in-page `#cf-turnstile` click fallback runs, which cannot reach a cross-origin widget checkbox. Have CLAIM_JS return the object directly (drop JSON.stringify) or parse the string via serde_json::from_str.
Existing Code
            if !claimed && start.elapsed().as_secs() < 4 {
                let res: Value = self.eval_js(CLAIM_JS).await.unwrap_or(Value::Null);
                if let Some(obj) = res.as_object() {
bug
medium
L722-L724
Check-then-act race: two concurrent callers (concurrent tasks are explicitly supported via tab_session) can both observe an empty `tabs` map and both attach+register a separate page target, silently creating a second tab and repointing `active`. Guard the attach with a dedicated mutex or re-check `tabs` after acquiring the lock inside the else branch.
Existing Code
        if !self.tabs.lock().await.is_empty() {
            return Ok(());
        }
bug
low
L2051-L2052
When `index` is out of range (or the element isn't an input), the fallback JS returns the string "not found"/"not an input", but click()/type_text()/hover() still return Ok(()). A stale element list therefore reports success for a no-op, and the LLM has no way to self-correct. Treat a non-'clicked'/'typed' result as an error, or have the JS throw.
Existing Code
                if (el) {{ el.click(); return 'clicked'; }}
                return 'not found';
bug
low
L1033-L1034
Network.setBlockedURLs REPLACES the whole block list for the session, and nothing ever resets it. After one navigation with `disable_resources`/`block_trackers`, the extended patterns stay active for every later navigation in that session even when default NavOpts are used — later pages silently get resources/trackers blocked. Either re-apply the base BLOCKED_URLS list when the opts are default, or document that blocking is sticky per session.
Existing Code
        if opts.disable_resources || opts.block_trackers {
            let mut urls: Vec<&str> = BLOCKED_URLS.to_vec();
security
high
L98
`ensure_key` performs a read-then-generate-then-write sequence with no atomicity. Two concurrent first runs each generate a different key, and whichever `write` lands last silently clobbers the other key — permanently locking out entries encrypted with the other key. Worse, *any* read failure (EACCES, transient I/O error) is treated as "no key yet" and triggers regeneration/overwrite of an existing key. Use `OpenOptions::new().write(true).create_new(true)` to create the key exclusively; on `AlreadyExists`, re-read the file instead of overwriting.
Existing Code
    std::fs::write(key_path(), key)?;
Suggested Change
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    match opts.open(key_path()) {
        Ok(mut f) => f.write_all(&key)?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let raw = std::fs::read(key_path())?;
            return raw.as_slice().try_into()
                .map_err(|_| anyhow::anyhow!("vault.key must be 32 bytes"));
        }
        Err(e) => return Err(e.into()),
    }
security
high
L102
The `set_permissions` result is discarded (`let _ = ...`). `std::fs::write` creates the file with default permissions (0o666 & ~umask, typically 0o644), so if the chmod fails the vault master key stays world-readable, and there is a window where the freshly written key is readable before chmod runs. Set 0o600 at creation time via `OpenOptionsExt::mode(0o600)` (see the `ensure_key` comment) and propagate the permission error instead of ignoring it.
Existing Code
        let _ = std::fs::set_permissions(key_path(), std::fs::Permissions::from_mode(0o600));
Suggested Change
        std::fs::set_permissions(key_path(), std::fs::Permissions::from_mode(0o600))?;
bug
medium
L171
`set`, `set_username`, and `remove` all follow a load-entries → mutate → save-whole-file pattern with no lock (thread or process level). Two concurrent mutations both read the same snapshot; the last `save_entries` overwrites the file and silently drops the entry the other caller persisted. Wrap the read-modify-write section in an exclusive file lock (e.g. fd-lock/fs2) or use per-entry merge semantics with an atomic replace.
Existing Code
    entries.retain(|e| !(e.service == service && e.profile == profile)); // upsert
bug
medium
L151
`save_entries` writes directly to the live `vault.json`. A crash or power loss mid-write can truncate/corrupt the index, and there is no backup or recovery path — the next mutating call sees the truncated file as empty (`load_entries` maps it to `Ok(Vec::new())`) and persists a partial index, losing all other entries. Write to a temp file in the same directory, fsync it, then atomically `rename` over `vault.json` (and fsync the directory on Unix).
Existing Code
    std::fs::write(index_path(), serde_json::to_string_pretty(entries)?)?;
Suggested Change
    let tmp = index_path().with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(entries)?)?;
    std::fs::rename(&tmp, index_path())?;
bug
medium
L145
`load_entries` maps *every* read failure (permission denied, I/O error, etc.) and zero-length/whitespace files to `Ok(Vec::new())`. Only `NotFound` should mean "empty vault"; other errors must propagate. As written, an unreadable or truncated index looks empty, so a subsequent `set`/`remove` rewrites the file and can permanently discard previously stored entries (compounding the non-atomic write issue).
Existing Code
        _ => Ok(Vec::new()),
Suggested Change
        Ok(_) => Ok(Vec::new()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
security
low
L260-L262
The hand-rolled base32 decoder skips `=` padding without validating placement/count and silently discards leftover bits when the input doesn't end on a byte boundary, so malformed seeds (wrong length, missing padding, non-zero trailing bits) are accepted as TOTP keys. It also enforces no minimum key size (RFC 4226 recommends ≥128-bit, ideally 160-bit, shared secrets). Validate padding, check that leftover bits are zero, and reject seeds shorter than 16 decoded bytes before enrollment.
Existing Code
        if matches!(c, ' ' | '\n' | '\r' | '-' | '=') {
            continue;
        }
bug
low
L307-L309
`fill_js` chains `replace("SEL", …)` then `replace("VAL", …)`. The second replacement scans the string produced by the first one, so if the *selector* value contains the substring `VAL`, the just-inserted selector JSON is corrupted and the generated script is broken (a user-configurable selector could trigger this). Replace the tokens in a single pass or use unambiguous placeholders (e.g. `format!` with a delimiter that cannot appear in user input) so data can never collide with a template token.
Existing Code
    FILL_JS
        .replace("SEL", &jstr(sel))
        .replace("VAL", &jstr(val))
bug
high
L2007-L2009
When `exclude_social` is false, `social` is empty and the generated selector list ends with a trailing comma (`...header,aside,`). `querySelectorAll` rejects a selector list with a trailing comma (SyntaxError), so the default clean-text path throws in-page and never returns content. Fix by prepending the leading comma to the social selectors so the list stays valid when empty.
Existing Code
    format!(
        r#"(()=>{{const w=document.createElement('div');w.innerHTML=document.body.innerHTML;for(const s of w.querySelectorAll('script,style,noscript,iframe,nav,footer,header,aside,{social}'))s.remove();const t=w.textContent||'';return t.split(/\s+/).filter(w=>w.length>={word_threshold}).join(' ').slice(0,8192);}})()"#
    )
Suggested Change
    let social = if exclude_social {
        ",[href*=\"facebook.com\"],[href*=\"twitter.com\"],[href*=\"instagram.com\"],[href*=\"linkedin.com\"],[href*=\"youtube.com\"]"
    } else {
        ""
    };
    format!(
        r#"(()=>{{const w=document.createElement('div');w.innerHTML=document.body.innerHTML;for(const s of w.querySelectorAll('script,style,noscript,iframe,nav,footer,header,aside{social}'))s.remove();const t=w.textContent||'';return t.split(/\s+/).filter(w=>w.length>={word_threshold}).join(' ').slice(0,8192);}})()"#
    )
bug
medium
L1280-L1283
`batch_map` indexes `urls[0]` when the capability probe or the single-target tab-open fails. All public batch entry points (batch_fetch/batch_markdown/batch_extract/batch_eval/batch_interact/batch_screenshot) therefore panic with an index-out-of-bounds on an empty `urls` slice. Add an early empty-input guard at the top of `batch_map`.
Existing Code
    let single = match browser.single_target_probe().await {
        Ok(s) => s,
        Err(e) => return vec![batch_err(&urls[0], e)],
    };
Suggested Change
    if urls.is_empty() {
        return vec![];
    }
    let single = match browser.single_target_probe().await {
        Ok(s) => s,
        Err(e) => return vec![batch_err(&urls[0], e)],
    };
bug
medium
L2255
For an empty `paths` slice, `workers` becomes `0.min(…).max(1) = 1` and the chunk size is `(0 + 1 - 1) / 1 = 0`; `paths.chunks(0)` panics ("chunk size must be non-zero"). `pdf_extract_batch(&[])` should return an empty vec instead of panicking.
Existing Code
        for chunk in paths.chunks((paths.len() + workers - 1) / workers) {
Suggested Change
    if paths.is_empty() {
        return vec![];
    }
    let n = std::thread::available_parallelism()
        .map(|x| x.get())
        .unwrap_or(4);
bug
medium
L484-L487
`avg.max(target.min(avg.max(target)))` simplifies algebraically to `max(avg, target)`: when `target > cur` (slow server), `avg < target` so the delay jumps straight to `target` instead of moving halfway — contradicting the "move halfway" comment. Only the speed-up direction is actually averaged. Use the plain average clamped to [floor, max] (or document the intended jump-to-target behavior).
Existing Code
            let avg = (cur + target) / 2;
            avg.max(target.min(avg.max(target)))
                .min(self.autothrottle_max_delay_ms)
                .max(floor)
Suggested Change
            let avg = (cur + target) / 2;
            avg.min(self.autothrottle_max_delay_ms).max(floor)
bug
medium
L633-L637
For an absolute href like `https://example.com/product/1`, `splitn(3, '/')` yields `["https:", "", "example.com/product/1"]`, so `nth(2)` returns `"example.com/product/1"` — the host is still in the string and a robots `Disallow: /product` prefix can never match. `respect_robots` therefore silently blocks nothing for all absolute links. Parse with `url::Url` and match against `u.path()`.
Existing Code
            let path = link
                .splitn(3, '/')
                .nth(2)
                .map(|p| p.to_lowercase())
                .unwrap_or_default();
Suggested Change
            let path = url::Url::parse(link)
                .map(|u| u.path().to_lowercase())
                .unwrap_or_default();
performance
medium
L609-L612
`crawl` is an async fn but performs a blocking `ureq::get(...).call()` on the runtime thread using the bare default agent (no global timeout configured), so a hung robots.txt server can stall the async worker indefinitely. The checkpoint save/load/delete also do synchronous filesystem I/O inside the async fn. Use the pooled `browser_agent()` (30s timeout) wrapped in `tokio::task::spawn_blocking` (or an async client), and move the checkpoint fs ops off the async path.
Existing Code
        let disallowed: Vec<String> = if self.respect_robots {
            ureq::get(&format!("{seed_origin}/robots.txt"))
                .call()
                .ok()
Suggested Change
        let disallowed: Vec<String> = if self.respect_robots {
            let url = format!("{seed_origin}/robots.txt");
            let resp = tokio::task::spawn_blocking(move || browser_agent().get(&url).call().ok()).await;
            resp.ok()
                .flatten()
security
high
L434-L435
Zip-slip path traversal: `f.name()` is joined directly onto `dest` with no sanitization. A crafted archive entry such as `../../escape` or an absolute path (which `Path::join` uses to *replace* the base) writes outside the extraction directory. Even though the current download sources are trusted, this is a network-fed extraction routine and a single compromised/malicious release becomes arbitrary file write. Reject absolute paths and any `..` component before joining.
Existing Code
        let name = f.name().to_string();
        let out = dest.join(&name);
Suggested Change
        let name = f.name().to_string();
        let rel = std::path::Path::new(&name);
        if rel.is_absolute()
            || rel.components().any(|c| matches!(c, std::path::Component::ParentDir))
        {
            anyhow::bail!("unsafe path in archive: {name}");
        }
        let out = dest.join(rel);
bug
high
L443-L444
`std::io::copy` creates files with default permissions (0644 & ~umask); the entry's Unix mode (`f.unix_mode()`) is never applied. The CfT linux64 zip stores an executable `chrome`, so after `install_chrome` on Linux the binary is not executable and the subsequent launch fails with EACCES (note `install_ffmpeg`/`install_whisper_bin` explicitly call `make_executable`, but `install_chrome`/`install_obscura` do not). Apply the entry mode after writing, or chmod the found binary in the installers.
Existing Code
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut f, &mut w)?;
Suggested Change
        let mut w = std::fs::File::create(&out)?;
        std::io::copy(&mut f, &mut w)?;
        #[cfg(unix)]
        if let Some(mode) = f.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))?;
        }
bug
high
L508
On macOS `bin_name("chrome")` returns `"chrome"`, but the CfT macOS archive ships `Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing` (as the module comment itself notes). `find_named` never matches, so `install_chrome` fails with "chrome binary not found after extract" and `find_cft_chrome` never returns the downloaded build on macOS — the download is wasted and system Chrome is always used. Use the platform-specific executable name on macOS (this also affects `find_cft_chrome`).
Existing Code
    find_named(&dest, &bin_name("chrome"), 4).with_context(|| {
Suggested Change
    let exe = if cfg!(target_os = "macos") {
        "Google Chrome for Testing"
    } else {
        &bin_name("chrome")
    };
    find_named(&dest, exe, 4).with_context(|| {
bug
high
L922-L925
The progress loop only terminates when `done >= total`. If a chunk permanently fails (`fetch_chunk` gives up after its 3 retries) or the server delivers fewer bytes than the probe advertised, all worker threads finish but `done` never reaches `total`, so this loop spins forever at 120 ms and the install hangs with no error ever surfaced. Track finished workers (e.g. an atomic counter incremented when each handle completes) and bail out of the loop — then propagate the worker error — instead of waiting only on byte counts.
Existing Code
        if d >= total {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(120));
bug
high
L817-L820
`.ok` is trusted without verifying `dest` still exists. Callers that delete the destination leave a stale marker behind: `install_llama_server` removes `_dl` but not `_dl.ok`, so a subsequent run (e.g. `--force`) makes `download_to_file` return early without recreating the file, and the caller's `std::fs::read(_dl)` fails with "No such file". The same hazard exists if a user deletes a downloaded model/archive manually. Guard the short-circuit with `ok.exists() && dest.exists()`.
Existing Code
    if ok.exists() {
        println!("  ✓ {name}: {} (already complete)", human_bytes(total));
        return Ok(());
    }
Suggested Change
    if ok.exists() && dest.exists() {
        println!("  ✓ {name}: {} (already complete)", human_bytes(total));
        return Ok(());
    }
security
medium
L420
The temp path is derived only from the process PID in the world-writable temp dir. Concurrent installs within one process collide on the same name (and a crash leaves `.part`/`.part.done`/`.ok` behind that are blindly reused — potentially resuming stale chunks from a *different* URL downloaded to the same name). A local attacker who can predict the PID can pre-create `<tmp>.part` as a symlink: `OpenOptions` follows it, `part.exists()` is true so `set_len` is skipped, and all chunk writes go through the symlink. Use a unique per-call name (counter/nonce) with O_CREAT|O_EXCL, and clean up all sidecars.
Existing Code
    let tmp = std::env::temp_dir().join(format!("webrain-dl-{}", std::process::id()));
Suggested Change
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!(
        "webrain-dl-{}-{nonce}",
        std::process::id()
    ));
bug
medium
L111-L114
Version directories are ordered lexicographically (reverse), so `chrome-99…` sorts above `chrome-100…` and `find_cft_chrome` returns an older cached build whenever multiple versions are installed; the same flaw affects `find_lightpanda`/`find_obscura` (e.g. `v0.9.0` vs `v0.10.0`). Compare the version numerically (parse the numeric component or use a version-aware comparator) rather than raw `OsString` ordering.
Existing Code
    entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
    for e in entries {
        let p = e.path();
        if p.is_dir() && e.file_name().to_string_lossy().starts_with("chrome-") {
bug
medium
L647-L649
Copy errors are discarded with `let _`, so a failed DLL/backend copy (e.g. a Windows `avcodec-*.dll` for ffmpeg, or `whisper.dll`/`ggml-cpu-*.dll` for whisper-cli) silently produces a broken install that is still reported as "ok". Propagate the error (`?` or collect). The same pattern also appears in `install_whisper_bin` and `copy_flat`.
Existing Code
        if p.is_file() {
            let _ = std::fs::copy(&p, dir.join(p.file_name().unwrap()));
        }
Suggested Change
        if p.is_file() {
            std::fs::copy(&p, dir.join(p.file_name().unwrap()))?;
        }
bug
low
L891
`last_pct` is declared immutable and never updated, so `pct != last_pct` is always true and the progress bar is redrawn on every 120 ms tick regardless of whether the percentage changed — defeating the intended throttle and producing constant stdout churn during multi-GB downloads. Make it `mut` and update it each iteration.
Existing Code
    let last_pct: i64 = -1;
bug
low
L863-L864
Each `fetch_chunk` attempt has no timeout (ureq's default is no timeout), so a stalled server leaves the worker thread blocked forever, and combined with the byte-count-only progress loop the install never completes. Configure a per-attempt connect/read timeout (with backoff) so failures surface instead of hanging.
Existing Code
                for attempt in 0..3 {
                    match fetch_chunk(&url, &part, start, end, &done) {
bug
high
L365-L368
The `.clamp(1, …)` lower bound makes zero frames impossible. `Detail::Transcript` (hard_cap 0) and `max_frames: Some(0)` are both clamped up to 1, so ffmpeg still runs and writes `frame_0001.jpg`. This affects local files directly (the early-return in `watch()` only skips frames for URL+Transcript with captions), violating the Transcript contract of "no frames". Clamp from 0 and bail early when the cap is 0.
Existing Code
    let cap = opts
        .max_frames
        .unwrap_or_else(|| frame_budget(probe.duration, opts.detail))
        .clamp(1, opts.detail.hard_cap().max(1));
Suggested Change
    let cap = opts
        .max_frames
        .unwrap_or_else(|| frame_budget(probe.duration, opts.detail))
        .clamp(0, opts.detail.hard_cap());
    if cap == 0 {
        return Ok(Vec::new());
    }
bug
high
L410-L414
The uniform-fps fallback rebuilds the ffmpeg command without the `-ss`/`-to` input options, so when scene-select yields <4 frames it decodes from t=0 and samples the entire video — producing frames outside the caller's `start`/`end` window and `pts_time` values inconsistent with the requested range. Reuse the same `-ss` (before `-i`) and `-to` options as the primary command, otherwise the user's time bounds are silently ignored.
Existing Code
        let fps = (cap as f64 / probe.duration.max(1.0)).min(2.0);
        let out = run(std::process::Command::new(tool_path("ffmpeg"))
            .args(["-hide_banner", "-loglevel", "info", "-y"])
            .arg("-i")
            .arg(path)
bug
medium
L613-L616
`extract_audio` produces a 16 kHz PCM WAV (`audio.wav`), but the multipart upload labels it `filename="audio.mp3"` with `Content-Type: audio/mpeg`. Providers that validate the extension/content-type (or run an MP3 decoder) can reject perfectly valid audio, forcing spurious failover to the next provider or failing transcription entirely. Send it as `audio.wav` with `audio/wav` (or `audio/x-wav`).
Existing Code
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.mp3\"\r\nContent-Type: audio/mpeg\r\n\r\n")
                .as_bytes(),
        );
Suggested Change
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n")
                .as_bytes(),
        );
bug
medium
L399-L403
Neither ffmpeg invocation in `frames()` checks `out.status` — `run()` returns the `Output` regardless of the exit code. A real extraction failure (undecodable file, unsupported option, older ffmpeg rejecting `-to` before `-i`, etc.) therefore yields `Ok` with zero frames, and `watch()`'s `unwrap_or_default()` silently turns it into a "successful" watch result with no error surfaced. Check `out.status.success()` after both the scene-select and fallback runs and return a contextual error (including stderr).
Existing Code
    cmd.arg("-q:v").arg("4");
    cmd.arg(&pattern);
    let out = run(&mut cmd)?;

    let mut extracted = parse_frame_paths(&outdir, &out.stderr);
Suggested Change
    cmd.arg("-q:v").arg("4");
    cmd.arg(&pattern);
    let out = run(&mut cmd)?;
    if !out.status.success() {
        return Err(anyhow!(
            "ffmpeg frame extraction failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let mut extracted = parse_frame_paths(&outdir, &out.stderr);
bug
high
L1143-L1146
`watch_batch` shares a single `&WatchOpts` across up to 8 workers, and with the default `out_dir: None` every `watch()` call derives the same `watch_<pid>` work dir (from `std::process::id()`, identical across threads). Workers then concurrently download to the same `download/` dir, call `clear_jpgs` on the same `frames/` dir, and overwrite the same `video.*`/`audio.wav`. Frames get deleted mid-extraction, frame paths/pts get mismatched across videos, and a transcript can be paired with another video's audio. Derive a per-source (or per-worker) work dir when batching, or require distinct `out_dir`s.
Existing Code
    let work = opts
        .out_dir
        .clone()
        .unwrap_or_else(|| format!("watch_{}", std::process::id()));
bug
medium
L1361-L1365
If any worker panics while holding the `results` guard (e.g. an unexpected panic inside the assignment), the mutex is poisoned and both `results.lock().unwrap()` in the workers and `results.into_inner().unwrap()` here will panic — turning one bad source into a total `watch_batch` panic instead of a per-source `{"error": …}` entry. Use `unwrap_or_else(|e| e.into_inner())` (and likewise for the `next` lock) so a poisoned batch still returns partial results.
Existing Code
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.unwrap_or_else(|| json!({"error": "watch worker failed"})))
Suggested Change
    results
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .map(|r| r.unwrap_or_else(|| json!({"error": "watch worker failed"})))
bug
high
L57
Byte-slicing a String at an arbitrary offset panics when the boundary falls in the middle of a multi-byte UTF-8 character. Any non-ASCII page whose text exceeds 2000 bytes will crash the CLI instead of printing. Use `floor_char_boundary` (stable since 1.73) or `get(..)` with a fallback.
Existing Code
println!("Text:  {}", &state.text[..state.text.len().min(2000)]);
Suggested Change
let end = state.text.floor_char_boundary(state.text.len().min(2000));
println!("Text:  {}", &state.text[..end]);
bug
medium
L256
The safety comment claims "single-threaded main", but `tokio::runtime::Runtime::new()` (multi-threaded) was called at the top of `main` and already spawned a worker pool before this line executes. In edition-2024 `std::env::set_var` is `unsafe` precisely because a concurrent env read (e.g. in tokio workers or any library doing DNS/TLS/env lookups) is a data race / UB. Either set the variable before the runtime is created, or better, pass the stealth-off flag through `SerpOpts` instead of a process-global env var.
Existing Code
unsafe { std::env::set_var("WEBRAIN_NO_STEALTH", "1") };
Suggested Change
// set before Runtime::new() / any thread spawn, or pass via SerpOpts
// instead of a process-global env var
bug
medium
L387
Errors are printed to stdout and the process still exits 0, so scripts/automation cannot detect a failed search. Worse, when `--json` is set the `error: ...` text is interleaved with the JSON document, producing invalid output for parsers. Print to stderr and propagate a non-zero exit.
Existing Code
Err(e) => println!("error: {e}"),
Suggested Change
Err(e) => {
    eprintln!("serp failed: {e}");
    return Err(e);
}
bug
medium
L304-L308
With `--json` and a browser engine, these launch/status messages are written to stdout *before* the JSON document, so the `--json` output for `brave`/`google` is not valid JSON. All diagnostics (launch notices, `--hold` prompt) should go to stderr when `json_out` is set, keeping stdout exclusively for the result document.
Existing Code
println!(
    "launched FRESH chrome (no cookies -> consent modal handled): {} (CDP_URL={})",
    launched.profile_dir.display(),
    launched.cdp_url
);
Suggested Change
if !json_out {
    println!(
        "launched FRESH chrome (no cookies -> consent modal handled): {} (CDP_URL={})",
        launched.profile_dir.display(),
        launched.cdp_url
    );
}
security
medium
L947-L952
`self_update` replaces the running executable with a binary downloaded from GitHub with no checksum/signature verification. A compromised release (or a MITM/bad TLS proxy — curl is invoked without CA pinning) results in arbitrary code execution on the next run. Verify the download against the published `.sha256` asset before renaming it into place.
Existing Code
let st = std::process::Command::new("curl")
    .arg("-fsSL")
    .arg("-o")
    .arg(&tmp)
    .arg(&url)
    .status()?;
Suggested Change
// fetch the .sha256 asset and verify digest before rename, e.g.:
let sum_url = format!("{url}.sha256");
let sum = std::process::Command::new("curl").arg("-fsSL").arg(&sum_url).output()?;
// compare sha256(tmp) against sum.stdout before std::fs::rename
maintainability
low
L671-L676
Round-trip inconsistency: `webrain cookies` without `--out` emits `{"count": N, "cookies": [...]}` while `webrain setcookies` only accepts a bare JSON array. A user piping the former into the latter silently gets "set 0 cookies". Emit the bare array on stdout (or accept the wrapped shape in `setcookies`).
Existing Code
None => println!(
    "{}",
    serde_json::to_string_pretty(
        &json!({"count": cookies.len(), "cookies": cookies})
    )?
),
bug
low
L65
The screenshot filename uses only epoch seconds, so two runs in the same second silently overwrite the previous screenshot. Include sub-second precision or a monotonic counter in the filename.
Existing Code
let out = format!("screenshot_{}.png", chrono_now());
Suggested Change
let out = format!("screenshot_{}_{}.png", chrono_now(), std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.subsec_millis())
    .unwrap_or(0));
security
low
L860-L862
`exe` (from `current_exe()`) is interpolated directly into a single-quoted PowerShell string without escaping. A path containing a `'` terminates the string and injects arbitrary PowerShell. Pass the path via an environment variable (`$env:WEBRAIN_EXE`) or escape `'` as `''` before interpolation.
Existing Code
let ps = format!(
    "Get-Process 'webrain*' -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -ne {self_pid} -and $_.Path -eq '{exe}' }} | Stop-Process -Force"
);
Suggested Change
let exe_esc = exe.replace('\'', "''");
let ps = format!(
    "Get-Process 'webrain*' -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -ne {self_pid} -and $_.Path -eq '{exe_esc}' }} | Stop-Process -Force"
);
bug
high
L551-L553
Index-out-of-bounds panic on ordinary provider output: `Endpoint::embed` never validates that the response contains exactly one vector per input (or that vectors are in input order). If the embedder returns fewer vectors than tiles (rate-limit, partial error, reordering), `vecs[i]` panics and takes down the caller. Validate `vecs.len() == inputs.len()` and fail with a `Result` error (also check all vectors share one dimension — see the `search`/`dot` zip-truncation risk) before indexing.
Existing Code
            for (i, t) in tiles.iter().enumerate() {
                store.add(&format!("{url}#tile{}", t.index), vecs[i].clone());
            }
Suggested Change
            if vecs.len() != tiles.len() {
                anyhow::bail!("embedder returned {} vectors for {} tiles", vecs.len(), tiles.len());
            }
            for (i, t) in tiles.iter().enumerate() {
                store.add(&format!("{url}#tile{}", t.index), vecs[i].clone());
            }
bug
medium
L84
Malformed embedding values are silently coerced: a non-numeric element becomes `0.0` here, and on `load` the same entries are silently dropped by `filter_map` / `unwrap_or_default`. A wrong-shaped provider response therefore produces a plausible but garbage index with no error. Validate that every element is a JSON number and that all vectors in the batch have the same length; return an error otherwise. Note `dot`/`search` also silently truncate to the shorter vector when dimensions differ.
Existing Code
                v.push(n.as_f64().unwrap_or(0.0) as f32);
bug
high
L593-L594
Mixed vector/caption store breaks retrieval. `has_text()` returns true whenever ANY caption exists, so a single offline `add_text` run forces `retrieve` into keyword-only mode and silently ignores the entire valid vector index. Worse, `add()` only touches `map` and `add_text()` only touches `texts` — neither removes the same id from the other collection — so once a store has any captions it can never return to embed mode, `save()` writes duplicate id lines (same id in both a `vec` and `text` record), and `len()` double-counts those ids. Make the two stores mutually exclusive per id (e.g., `add` removes the id from `texts` and vice-versa) or have `retrieve` merge results by mode instead of gating on `has_text()`.
Existing Code
    if store.has_text() {
        let top = store.search_text(query, k);
bug
medium
L233
Non-atomic index rewrite: `std::fs::write` truncates the live JSONL store in place, so a crash or I/O error mid-write corrupts the whole index (all vectors lost / partial lines). The preceding `create_dir_all(dir).ok()` also swallows permission/disk errors, leaving a confusing downstream failure. Write to a temporary file in the same directory and `rename` it into place atomically, and propagate `create_dir_all` errors instead of `.ok()`-ing them.
Existing Code
        std::fs::write(&self.path, s)?;
Suggested Change
        let tmp = self.path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(&tmp, &self.path)?;
performance
medium
L549
Blocking work on the async executor: `index_current_page` is `async` but runs the blocking `ureq` embed call (`client.embed(&inputs)`), the blocking `describe_tiles`/`caption_tiles` HTTP calls (which can also `std::thread::sleep` 5s on retry), and synchronous `store.save()` file IO directly on the executor thread. This stalls the runtime and defeats cancellation. Wrap these sections in `tokio::task::spawn_blocking` (or make this function synchronous and call it from a blocking context).
Existing Code
    let (mode, vision): (&str, Option<String>) = match client.embed(&inputs) {
performance
medium
L476-L477
This 5-second blocking sleep (plus the retried `post_vision`, itself blocking with its own internal 800ms sleep) runs on the async executor whenever `index_current_page` takes the offline caption path — the runtime stalls for the full retry window and the call cannot be cancelled. Move the whole caption path off the executor (spawn_blocking) or make the retry asynchronous with a bounded, cancellable delay.
Existing Code
        if raw.is_err() {
            std::thread::sleep(std::time::Duration::from_secs(5));
performance
medium
L386
`ask_viewport` is `async` but calls the fully synchronous `post_vision` (blocking ureq, up to the 120s agent timeout, plus internal thread sleeps on retry) directly on the executor, so one slow/dead provider stalls the entire runtime and the future can't be cancelled. Also note each provider in the chain is tried in a loop with no await point, so the whole failover is effectively serial blocking. Spawn this work on a blocking task or use an async HTTP client.
Existing Code
        match crate::video::post_vision(&agent, &endpoint, auth.as_deref(), &body) {
security
medium
L330-L331
Privacy/security: `ask_viewport` uploads full viewport or tile PNG screenshots (potentially containing PII, credentials, emails) to OpenRouter/OpenAI/Fireworks/Groq based solely on the presence of env keys, with no user-facing consent gate. Because `content` is captured once and cloned into every request, each provider in the chain receives the images even when a previous provider fails — there is no opt-out per provider. Consider requiring an explicit opt-in flag/parameter for cloud upload, or defaulting to the bundled local Qwen3-VL and only falling to cloud on explicit request.
Existing Code
    let content: Value = if !tiles.is_empty() {
        let mut c: Vec<Value> = vec![json!({"type":"text","text": prompt})];
bug
low
L606
Silent garbage on empty embed response: if the endpoint returns an empty `data` array, `q` becomes an empty vector, `norm(q) == 0`, and `search` yields all-zero scores for every entry — retrieval reports a successful JSON response with meaningless results. Return an explicit error when the embedder returns no vector for the query.
Existing Code
    let q = vecs.into_iter().next().unwrap_or_default();
Suggested Change
    let q = vecs
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("embedding endpoint returned no vector for query"))?;
performance
medium
L597
`crate::engines::serp_http_get` is a synchronous (blocking) network call executed directly inside this async function. On the MCP HTTP/stdio server this stalls the executor worker thread for the whole round-trip, and since `auto` fans out via `join_all`, blocking calls serialize instead of running concurrently. Wrap the call in `tokio::task::spawn_blocking` (or switch to an async HTTP client such as reqwest) so I/O doesn't block the executor.
Existing Code
match crate::engines::serp_http_get(url, opts.proxy.as_deref()) {
performance
medium
L914
Same blocking-I/O-in-async issue: `serpapi_google` calls the synchronous `serp_http_get` directly in an async context, stalling the executor thread for the network round-trip. Wrap in `tokio::task::spawn_blocking` or use an async client.
Existing Code
let (status, body) = crate::engines::serp_http_get(u.as_str(), opts.proxy.as_deref())?;
bug
medium
L906-L910
`serpapi_google` never sends SERPAPI's `start` offset parameter, so `opts.page` is silently ignored whenever this path is used (serpapi_first for limit>10, or the post-browser fallback). A paginated request (`--page 1`, `--page 2`) returns page-0 results again, duplicating results the caller already has. Add a `start` param derived from `opts.page` (SERPAPI honors `start` up to `num`, e.g. `start = page * num`) to make pagination consistent with the other engines.
Existing Code
        p.append_pair("engine", "google");
        p.append_pair("q", &opts.query);
        p.append_pair("num", &opts.limit.clamp(1, 100).to_string());
        p.append_pair("hl", lang);
        p.append_pair("gl", cc);
Suggested Change
        p.append_pair("engine", "google");
        p.append_pair("q", &opts.query);
        p.append_pair("num", &opts.limit.clamp(1, 100).to_string());
        p.append_pair("start", &(opts.page * opts.limit.clamp(1, 100)).to_string());
        p.append_pair("hl", lang);
        p.append_pair("gl", cc);
bug
medium
L98
`brave` is listed in HTTP_ENGINES even though it is a JS SPA that only renders in a browser. Consequently the `auto` merge and `fallback_chain` both call `http_search("brave", ...)`, which plain-HTTP fetches the SPA shell and always parses to zero results — the attached browser is never used for Brave on these paths. Worse, in the `auto` arm `http_search` returning `Ok(empty)` isn't recorded in `skipped` (only `Err` is), so the response silently claims Brave was included when it contributed nothing. Exclude `brave` from HTTP_ENGINES and route it through `browser_search` when a backend is attached (or record zero-result engines in `skipped`).
Existing Code
const HTTP_ENGINES: [&str; 4] = ["bing", "duckduckgo", "brave", "google"];
bug
low
L403-L405
`href.strip_prefix('/')` also matches protocol-relative links (`//host/path`), yielding `https://www.google.com//host/path`, which the caller then filters out as an internal google.com link — a legitimate external result is silently dropped before `normalize_url` ever gets a chance to handle it correctly. Check the `//` case before the single-slash case (or hand protocol-relative URLs to `normalize_url` first).
Existing Code
    if let Some(rest) = href.strip_prefix('/') {
        return format!("https://www.google.com/{rest}");
    }
Suggested Change
    if let Some(rest) = href.strip_prefix("//") {
        let rest = rest.split('#').next().unwrap_or(rest);
        return format!("https://{rest}");
    }
    if let Some(rest) = href.strip_prefix('/') {
        return format!("https://www.google.com/{rest}");
    }
bug
low
L1036-L1042
This post-browser SERPAPI fallback runs unconditionally, even when `opts.fallback` is false, contradicting the documented `SerpOpts.fallback` semantics ("Allow provider fallback when a specific engine errors or returns zero"). A caller that explicitly disables fallback still silently gets results from serpapi.com. Guard the block with `opts.fallback`, or document that SERPAPI is an intrinsic part of the google provider rather than a fallback.
Existing Code
    if serpapi_ready && !serpapi_first {
        if let Ok(rs) = serpapi_google(opts).await {
            if !rs.is_empty() {
                return Ok((rs, Vec::new()));
            }
        }
    }
Suggested Change
    if opts.fallback && serpapi_ready && !serpapi_first {
        if let Ok(rs) = serpapi_google(opts).await {
            if !rs.is_empty() {
                return Ok((rs, Vec::new()));
            }
        }
    }
bug
medium
L90
webrain_eval_in_frame is fully implemented in call_tool() and advertised in AGENT_GUIDE (which even counts "17 TOOLS"), but it is never added to list_tools() — only 16 tools are advertised. MCP clients can only invoke tools returned by list_tools(), so this documented tool is undiscoverable through the standard protocol and the guide's tool count is wrong. Either add a list_tools() entry (url_contains + js params, mirroring the call_tool arm) or remove the stale guide lines.
Existing Code
  webrain_eval_in_frame  run JS inside a cross-origin iframe (isolated world) →
performance
medium
L1429
Synchronous, potentially long-running I/O is executed directly inside the async dispatcher: http_fetch (network), sitemap_urls (network), validate_urls (network), download_files (disk/network), and vault::list/vault::get (disk) all block the executor thread. Under a single-threaded runtime (common for MCP servers) this stalls every other in-flight tool call, and even on a multi-threaded runtime it ties up worker threads. Route these helpers through tokio::task::spawn_blocking (or make them async) in the webrain_fetch_http / webrain_sitemap / webrain_validate_urls / webrain_download / webrain_profiles / webrain_login arms.
Existing Code
            match http_fetch(url) {
bug
medium
L588-L600
probe_needs_js relies on visible_text_len, but this tag-stripper counts the BODIES of <script>/<style>/<noscript> elements as "visible text" (anything between '>' and '<'). A JS shell whose logic lives in a long inline <script> can therefore blow past the 100-char threshold and be classified as needing no browser, even though the page renders nothing without JS — exactly the case the heuristic is meant to catch. Skip script/style/noscript element bodies when counting.
Existing Code
pub(crate) fn visible_text_len(html: &str) -> usize {
    let mut out = 0usize;
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out += 1,
            _ => {}
        }
    }
    out
}
Suggested Change
pub(crate) fn visible_text_len(html: &str) -> usize {
    let mut out = 0usize;
    let mut in_tag = false;
    let mut skip = false;
    let mut tag = String::new();
    for ch in html.chars() {
        if in_tag {
            if ch == '>' {
                in_tag = false;
                let t = tag
                    .trim_start_matches('/')
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !skip && (t == "script" || t == "style" || t == "noscript") {
                    skip = true;
                } else if skip && tag.trim_start().starts_with('/') {
                    skip = false;
                }
                tag.clear();
            } else {
                tag.push(ch);
            }
        } else if ch == '<' {
            in_tag = true;
        } else if !skip {
            out += 1;
        }
    }
    out
}
security
medium
L1968
The search query is spliced into the URL with only space→'+' substitution; characters such as &, #, ?, %, + are inserted raw. A query containing '&' silently injects extra URL parameters (and a query like "C++" produces an ambiguous "C+++"), corrupting the request or altering what the engine receives. Percent-encode the query (e.g. urlencoding::encode / url::form_urlencoded) instead of this manual replace.
Existing Code
            let encoded = q.replace(' ', "+");
bug
medium
L331
webrain_batch advertises op=eval ("custom JS extractor") and run_batch reads args["js"], erroring when it is empty — but the inputSchema for webrain_batch does not declare a `js` property. A schema-driven LLM client cannot discover how to pass the extractor code, so op=eval is effectively unusable through the documented surface. Add the `js` property to the batch schema.
Existing Code
                "op": {"type": "string", "enum": ["fetch","extract","interact","eval","screenshot"]},
Suggested Change
                "op": {"type": "string", "enum": ["fetch","extract","interact","eval","screenshot"]},
                "js": {"type": "string", "description": "eval: JS extractor to run in each tab (required when op=eval)"}
bug
medium
L282
The interact tool advertises action=drag ("trusted slider/drag CAPTCHAs") and the executor's webrain_drag arm reads x1/y1/x2/y2 — but those four parameters are missing from the inputSchema, which only defines x/y (for click_coords). Schema-driven clients have no way to learn the drag coordinates, so the drag action is undiscoverable and effectively unusable. Add x1, y1, x2, y2 to the interact schema.
Existing Code
                "action": {"type": "string", "enum": ["click","click_coords","drag","type","press","scroll","nav","tab","select","hover","check","dialog","wait","upload","dismiss_overlays","add_init_script"]},
Suggested Change
                "action": {"type": "string", "enum": ["click","click_coords","drag","type","press","scroll","nav","tab","select","hover","check","dialog","wait","upload","dismiss_overlays","add_init_script"]},
                "x1": {"type": "number", "description": "drag: start x"},
                "y1": {"type": "number", "description": "drag: start y"},
                "x2": {"type": "number", "description": "drag: end x"},
                "y2": {"type": "number", "description": "drag: end y"}
security
high
L898-L904
Content-Length from the client is trusted with no upper bound. A malicious/broken client can declare a huge Content-Length and stream data; this inner loop keeps growing `buf` until memory exhaustion — the `buf.len() > 64 * 1024` guard only covers the header phase (before the terminator is found) and never the body read. Cap `content_length` (reject values above a few MB) before reading the body, and bound the incremental reads.
Existing Code
            while buf.len() < head_end + 4 + content_length {
                let m = socket.read(&mut tmp).await?;
                if m == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..m]);
            }
bug
medium
L224
Only spaces are replaced with '+'; characters like `&`, `#`, `%`, `?`, `=` in `q` are inserted raw into the search URL, producing a malformed or wrong query (e.g. `&` starts a new parameter, `#` truncates the query). Use proper percent/form-encoding (application/x-www-form-urlencoded) instead of a manual space replace.
Existing Code
                let encoded = q.replace(' ', "+");
security
high
L267-L270
`std::env::set_var` mutates process-global state from a handler that runs concurrently across many connections. In edition 2024 this is `unsafe` precisely because concurrent `std::env::var`/`set_var` from other threads is a data race (UB) — e.g. webrain_login reads WEBRAIN_USER/PASS from env concurrently. It also permanently disables stealth injection for every other session/browser in the process with no reset. Pass the stealth flag via backend/launch configuration instead of process env.
Existing Code
                if engine == "google" {
                    // edition-2024 unsafe; mirrors webrain-cli's serp arm.
                    unsafe { std::env::set_var("WEBRAIN_NO_STEALTH", "1") };
                }
bug
medium
L465-L469
`as u16` silently truncates ports > 65535 (e.g. 70000 → 4464), so a bad `port` argument makes the tool connect to an unintended local port. The identical pattern is repeated in webrain_login and webrain_save_state/restore_state. Validate with `u16::try_from(...)` and return a tool error on overflow.
Existing Code
                let port: u16 = arguments
                    .get("port")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u16)
                    .unwrap_or(9222);
performance
high
L204
`watch_from_args` shells out to yt-dlp/ffmpeg/ffprobe, which can run for minutes. Called synchronously on the tokio worker (while also holding the session backend mutex across the whole request), it stalls every other connection scheduled on that worker. Run it via `tokio::task::spawn_blocking` like the webrain_pdf_extract path does.
Existing Code
                let result = crate::tools::watch_from_args(&arguments);
performance
medium
L113
`http_fetch` is a synchronous ureq call invoked directly in the async handler, blocking the tokio worker for the entire network fetch. The same pattern is used in webrain_search and the HTTP branches of webrain_serp. Wrap in `tokio::task::spawn_blocking` or use an async HTTP client so unrelated sessions aren't stalled.
Existing Code
                let result = match webrain_core::engines::http_fetch(url) {
performance
medium
L377
`pdf_images` runs CPU-bound lopdf parsing directly on the async worker, whereas the sibling `webrain_pdf_extract` correctly uses `spawn_blocking`. Move this call into `spawn_blocking` as well.
Existing Code
                let result = match webrain_core::engines::pdf_images(path, pages.as_deref()) {
performance
medium
L427
`vault::list()` (and the `std::fs::create_dir_all` / `std::fs::read` / `std::fs::write` in webrain_save_state/restore_state) are synchronous filesystem/decryption calls on the async handler path. Wrap in `spawn_blocking` or use `tokio::fs` so concurrent HTTP sessions aren't serialized behind disk I/O.
Existing Code
                let result = match webrain_core::vault::list() {
bug
medium
L192-L198
`isError` is hardcoded `false` here, but `result` can carry `status: "error"` (e.g. the ytdlp branch failing). MCP clients key off `isError` to surface failures, so errors are silently swallowed. Compute it like the other branches: `result.get("status").and_then(|v| v.as_str()) == Some("error")`.
Existing Code
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}],
                        "isError": false
                    }
                });
maintainability
medium
L917-L922
Every `initialize` mints a new session entry that is never reaped — sessions (and their CDP backends) accumulate for the lifetime of the HTTP server. A long-running server leaks memory and open browser connections; add a TTL/expiry or bound the map size, or reap sessions on connection close.
Existing Code
                state
                    .sessions
                    .lock()
                    .await
                    .insert(id.clone(), Arc::new(Default::default()));
                id
maintainability
medium
L296-L299
`std::mem::forget(l)` leaks the launch guard before the fallible `connect_with_url`. If the attach fails (or on server shutdown), the guest Chrome process is never closed and no registry tracks it — repeated connect failures accumulate orphan Chrome processes. Only forget the guard after a successful attach, and register it for cleanup like webrain_launch does via tools::store_launched.
Existing Code
                                Ok(l) => {
                                    let url = l.cdp_url.clone();
                                    // Warm session — keep the guest alive between calls.
                                    std::mem::forget(l);
bug
high
L10
`git archive HEAD:skills/webrain` packages only the state committed at HEAD. Any new, modified, or untracked files under skills/webrain are silently excluded, so the produced dist/webrain.skill can be stale or incomplete with no warning (while the header comment promises the current skill). Add a pre-flight check such as `git status --porcelain -- skills/webrain` and abort (or at least warn) when the working tree differs from HEAD, so the user isn't uploading an outdated bundle.
Existing Code
git archive --format=zip --prefix=webrain/ --output=dist/webrain.skill HEAD:skills/webrain 2>/dev/null \
Suggested Change
if git status --porcelain -- skills/webrain | grep -q .; then
  echo "warning: uncommitted changes in skills/webrain are NOT included in the bundle" >&2
fi
bug
medium
L11
All stderr from git archive is discarded (`2>/dev/null`) and every failure is reported as "not a git clone". Real failure causes — the `skills/webrain` path missing from HEAD, an invalid ref, or filesystem/permission problems — are all masked behind a misleading message, making diagnosis difficult. Verify the repo state separately with `git rev-parse --is-inside-work-tree`, and let the real git archive error or exit status surface instead of swallowing it.
Existing Code
  || { echo "not a git clone — git archive needs the repo (commit the skill first)"; exit 1; }
Suggested Change
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "not a git clone — git archive needs the repo" >&2; exit 1
fi
git archive --format=zip --prefix=webrain/ --output=dist/webrain.skill HEAD:skills/webrain \
  || { echo "git archive failed — is skills/webrain committed at HEAD?" >&2; exit 1; }
maintainability
medium
L10
The header comment declares the contract "one SKILL.md + scripts/, nothing else", but `git archive` blindly includes every tracked file under skills/webrain, and nothing validates the result — extra files, or a missing SKILL.md, would be bundled (or produced) without detection. After archiving, validate the layout: list the zip (`unzip -l`) and assert only `webrain/SKILL.md` and `webrain/scripts/*` are present, and run an integrity check like `unzip -t` before echoing "wrote".
Existing Code
git archive --format=zip --prefix=webrain/ --output=dist/webrain.skill HEAD:skills/webrain 2>/dev/null \
Suggested Change
unzip -t dist/webrain.skill >/dev/null || { echo "invalid archive produced"; exit 1; }
security
medium
L32
No integrity verification: the binary is downloaded and `chmod +x`'ed (and later executed via `webrain`) without any SHA-256 checksum or signature check. A tampered release asset or a compromised release would be installed silently. Note that the release workflow only publishes raw binaries with no checksums file, so nothing is verified here. Recommendation: publish a `checksums.txt` in the release and verify it (e.g., `curl -fsSL "$URL.sha256" ...` then compare with `sha256sum`/`shasum -a 256`) before installing, and at minimum abort if the downloaded file is empty/zero bytes.
Existing Code
URL="https://github.com/$REPO/releases/latest/download/$ASSET"
bug
medium
L34-L38
Non-atomic overwrite of an existing install: `curl -o` opens the final destination directly, truncating any previously installed `webrain` binary before the download completes. If the download is interrupted or fails partway (or two installer runs race), the old binary is destroyed and a truncated/corrupt file is left at `$INSTALL_DIR/webrain` that will fail at runtime. Fix: download to a temporary file in the same directory, then atomically `mv` it into place (rename is atomic on the same filesystem), removing the temp file on failure.
Existing Code
if ! curl -fsSL "$URL" -o "$INSTALL_DIR/webrain"; then
  echo "webrain: download failed. Check your network or the release at https://github.com/$REPO/releases" >&2
  exit 1
fi
chmod +x "$INSTALL_DIR/webrain"
Suggested Change
TMP="$INSTALL_DIR/.webrain.tmp.$$"
if ! curl -fsSL "$URL" -o "$TMP"; then
  rm -f "$TMP"
  echo "webrain: download failed. Check your network or the release at https://github.com/$REPO/releases" >&2
  exit 1
fi
chmod +x "$TMP"
mv -f "$TMP" "$INSTALL_DIR/webrain"
maintainability
low
L40-L42
PATH detection only matches the exact literal substring `:$INSTALL_DIR:`. If `WEBRAIN_INSTALL_DIR` is set with a trailing slash, or the install directory is reachable via a symlink already present in `$PATH`, the check falsely reports the directory as missing and prints a misleading hint. Normalize `INSTALL_DIR` (strip trailing slashes) and/or also accept the trailing-slash form in the pattern.
Existing Code
case ":$PATH:" in
  *":$INSTALL_DIR:"*) : ;;
  *) echo "webrain: installed to $INSTALL_DIR/webrain — add $INSTALL_DIR to your PATH." >&2 ;;
Suggested Change
INSTALL_DIR="$(printf '%s' "$INSTALL_DIR" | sed 's:/*$::')"
case ":$PATH:" in
  *":$INSTALL_DIR:"*|*":$INSTALL_DIR/:"*) : ;;
  *) echo "webrain: installed to $INSTALL_DIR/webrain — add $INSTALL_DIR to your PATH." >&2 ;;
maintainability
medium
L12
Declaring `features = ["full"]` at the workspace level forces the entire tokio feature set onto every member that inherits it via `tokio = { workspace = true }` (webrain-core, webrain-mcp, webrain-cli all inherit without per-member feature overrides). Since webrain-core is a library, this full feature set also leaks into any downstream consumer through Cargo feature unification, and it measurably increases compile time. Prefer declaring only the features each crate actually needs, either by moving feature selection into the member manifests (`tokio = { workspace = true, features = [...] }`) or by narrowing this shared declaration.
Existing Code
tokio = { version = "1", features = ["full"] }
Suggested Change
tokio = { version = "1" }
maintainability
low
L6-L9
The shared `[workspace.package]` metadata lacks `repository` and `description`, both of which crates.io expects for published crates (a missing `description` triggers `cargo package` warnings and hurts discoverability, and there is no link back to the source). Since all members inherit this table via `version.workspace = true` / `edition.workspace = true`, add these fields here so every published member gets them consistently (e.g. a `description` and a `repository` URL for the project).
Existing Code
version = "0.7.3"
edition = "2024"
license = "MIT"
rust-version = "1.85"
Suggested Change
version = "0.7.3"
edition = "2024"
license = "MIT"
rust-version = "1.85"
description = "WebRain: multi-crate web research agent toolkit"
repository = "https://github.com/<owner>/webrain"
maintainability
medium
L13-L14
These two dependencies are pinned directly instead of going through `[workspace.dependencies]`, unlike every other dependency in this manifest. `base64` is also pinned (with the same version) in webrain-core/Cargo.toml, so two copies of the version constraint now live in different files and can silently drift. In a repo that consistently uses workspace inheritance (root Cargo.toml defines workspace.dependencies for all other deps), move these into `[workspace.dependencies]` and reference them with `workspace = true`. This also centralizes MSRV compatibility checks: the workspace declares `rust-version = "1.85"` with resolver = "3" (MSRV-aware), and tiktoken-rs 0.12.0's own MSRV should be verified against that floor before pinning it here.
Existing Code
base64 = "0.22"
tiktoken-rs = "0.12.0"
Suggested Change
base64 = { workspace = true }
tiktoken-rs = { workspace = true }
maintainability
medium
L2-L4
The package inherits only `version` and `edition` from `[workspace.package]`, but the workspace root also defines `license = "MIT"` and `rust-version = "1.85"`. Because webrain-mcp is a library crate (src/lib.rs) in a workspace with an explicit MSRV policy and an MSRV-aware resolver (resolver = "3"), it should also inherit `rust-version` (so Cargo selects dependency versions compatible with the project's MSRV) and `license` (cargo will warn on publish/packaging otherwise). Consider `license.workspace = true` and `rust-version.workspace = true` alongside the existing fields.
Existing Code
name = "webrain-mcp"
version.workspace = true
edition.workspace = true
Suggested Change
name = "webrain-mcp"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
maintainability
medium
L1-L4
Missing publication metadata and no publish guard. The workspace root defines `license = "MIT"` and `rust-version = "1.85"` under `[workspace.package]`, but this crate only inherits `version` and `edition`. If this binary crate is ever published, an accidental `cargo publish` would produce a crate with no `description`, `license`, or `repository`, and cargo would refuse/abort or publish incomplete metadata. Since this is a CLI binary (not a library), the safest fix is to add `publish = false`; alternatively inherit `license`/`rust-version` (via `license.workspace = true`, `rust-version.workspace = true`) and add `description` and `repository`.
Existing Code
[package]
name = "webrain-cli"
version.workspace = true
edition.workspace = true
Suggested Change
[package]
name = "webrain-cli"
version.workspace = true
edition.workspace = true
publish = false
maintainability
low
L11-L12
Path-only dependencies without version requirements. `webrain-core` and `webrain-mcp` are declared only with `path`, so if this crate were ever published to crates.io they could not be resolved by external consumers (crates.io does not accept path-only dependencies for published crates). If publishing is intended, add `version` requirements matching the published versions of those crates; otherwise, this is fine for a workspace-internal CLI but should be combined with `publish = false` to prevent accidental publication.
Existing Code
webrain-core = { path = "../webrain-core" }
webrain-mcp = { path = "../webrain-mcp" }
maintainability
low
L18
`rpassword` is the only external dependency with an inline version requirement while all other dependencies use workspace inheritance. Move it to `[workspace.dependencies]` in the root manifest and reference it as `rpassword = { workspace = true }` to centralize version management and avoid version drift in this multi-crate repository.
Existing Code
rpassword = "7"
Suggested Change
rpassword = { workspace = true }
maintainability
medium
L1-L5
Missing publish/MSRV metadata. The workspace already declares `rust-version = "1.85"` and inherits version/edition/license, but this crate neither inherits `rust-version` nor declares `description`/`repository`/`readme`. `cargo publish` requires a `description`, and without `rust-version` downstream users aren't warned that the code needs Rust ≥ 1.85. Suggest adding `rust-version.workspace = true` plus a `description` (and `repository`/`readme`/`include` if this crate is published).
Existing Code
[package]
name = "webrain-core"
version.workspace = true
edition.workspace = true
license.workspace = true
maintainability
medium
L33
Heavy dependencies are all unconditional. `scraper`, `htmd`, `image`, `lopdf`, `pdf-inspector`, `flate2`, `zip`, `ureq`, and `regex` are always compiled into every consumer of this library even though the `default` feature set is intentionally empty. Only `pdfium` is optional. Consider feature-gating the parsing stacks (e.g. `serp` for scraper, `markdown` for htmd, `pdf` for pdf-inspector/lopdf/flate2, `images` for image) so the base crate stays lean and downstream build times/binary size don't balloon.
Existing Code
scraper = "0.24"
maintainability
medium
L42
Direct `lopdf = "0.44"` duplicates the transitive `lopdf 0.41.0` that `pdf-inspector 0.1.7` already pulls in — Cargo.lock resolves both versions, so two complete PDF parsing stacks (and their duplicated aes/flate2/sha2/getrandom deps) are compiled. The comment already marks this as a stopgap until pdf-inspector exposes image XObjects natively; until then, pinning the direct dep to `0.41` (matching pdf-inspector) would avoid the duplicate build.
Existing Code
lopdf = "0.44"
maintainability
low
L15
`tokio = { workspace = true }` inherits `features = ["full"]` from the workspace, forcing every downstream consumer of this library to compile the entire tokio feature set (net, rt-multi-thread, time, io-util, macros, …). For a library it's better to depend on tokio with `default-features = false` plus only the features actually used (e.g. `rt`, `net`, `time`, `io-util`) and leave the `full` profile to the binaries in the workspace.
Existing Code
tokio = { workspace = true }
other
medium
L10-L11
The `dependencies` label referenced here (and in the github-actions entry below) is not defined anywhere in the repository — no label config file or label-management workflow was found. Dependabot does not create labels; it only applies labels that already exist in the repository. If this label hasn't been created in the repo's Settings → Labels, Dependabot will silently skip labeling the PRs (and any automation filtering on it will miss them). Verify the label exists, or remove this `labels` block / create the label.
Existing Code
    labels:
      - dependencies
security
medium
L25
Third-party action is pinned to the mutable `stable` tag, which is a floating reference that gets updated whenever the action maintainer releases a new version. This makes CI non-reproducible and introduces a supply-chain risk (a tag can be re-pointed or hijacked). Pin to the full commit SHA of a specific release and use Dependabot/Renovate to manage updates.
Existing Code
      - uses: dtolnay/rust-toolchain@stable
Suggested Change
      - uses: dtolnay/rust-toolchain@<full-commit-sha>
security
medium
L28
Third-party action is pinned to the mutable `v2` tag. Major-version tags can be silently re-pointed to new releases, breaking reproducibility and enabling supply-chain tampering. Pin to the full commit SHA of the exact release you intend to use.
Existing Code
      - uses: Swatinem/rust-cache@v2
Suggested Change
      - uses: Swatinem/rust-cache@<full-commit-sha>
other
medium
L10-L14
No `concurrency` group is defined, so `push` and `pull_request` runs on main can overlap (e.g., a push plus its own PR, or rapid consecutive pushes), producing redundant CI runs that waste runner time and can cause confusing failure states. Add a concurrency group keyed by workflow/ref with `cancel-in-progress: true` so stale runs are cancelled.
Existing Code
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]
Suggested Change
concurrency:
  group: ci-${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
other
medium
L22
The job has no `timeout-minutes`. A hung `cargo` command (network stall, dependency lock contention, deadlock in tests) will hold the runner until the platform default (6 hours on GitHub-hosted runners), wasting CI resources. Set an explicit job-level timeout (e.g., 30–60 minutes) so runaway runs are killed early.
Existing Code
    runs-on: ubuntu-latest
Suggested Change
    runs-on: ubuntu-latest
    timeout-minutes: 60
bug
high
L15-L16
The MCP HTTP server binds to `127.0.0.1:<port>` inside the container (see `webrain-cli/src/main.rs`: `let addr = format!("127.0.0.1:{port}")`). Docker's published port forwards to the container's bridge IP (e.g. 172.17.0.2), not to the container loopback, so connections to `host:9223` will be refused — the MCP endpoint is effectively unreachable. Make the bind address configurable (e.g. `WEBRAIN_HTTP_ADDR=0.0.0.0:9223`) and use it in the container, or drop the `ports` mapping if the endpoint is only needed inside the container.
Existing Code
    ports:
      - "9223:9223" # HTTP MCP transport (webrain mcp --http 9223)
security
medium
L15-L16
If the bind issue is fixed so the endpoint becomes reachable, publishing on all host interfaces (`0.0.0.0:9223`) exposes an unauthenticated browser-automation JSON-RPC endpoint to the LAN. The docs describe this transport as localhost-only (`http://127.0.0.1:9223/mcp`) and sessions are identified only by sequential IDs with no auth. Restrict the mapping to `127.0.0.1:9223:9223` unless remote clients are explicitly intended and authenticated.
Existing Code
    ports:
      - "9223:9223" # HTTP MCP transport (webrain mcp --http 9223)
maintainability
low
L14
Hardcoding `container_name: webrain` prevents running multiple instances of this service on the same host and causes a name conflict if a container named `webrain` already exists (e.g. created via `docker run --name webrain`). Consider removing `container_name` and letting Compose derive a unique name, or parameterize it, e.g. `${CONTAINER_NAME:-webrain}`.
Existing Code
    container_name: webrain
Suggested Change
    container_name: ${CONTAINER_NAME:-webrain}
security
medium
L29
Third-party action is pinned to a mutable major tag (`v6`) instead of a full commit SHA. Tags can be force-moved or overwritten, allowing a compromised release to run arbitrary code with this workflow's `pull-requests`/`statuses` write token. Pin to the full commit SHA of the reviewed release (e.g., `amannn/action-semantic-pull-request@<full-sha>`) and note the corresponding version in a comment.
Existing Code
        uses: amannn/action-semantic-pull-request@v6
security
medium
L85
Same supply-chain concern as the lint action: `marocchino/sticky-pull-request-comment@v3` is a mutable tag. Pin it to a full commit SHA to prevent tag hijacking, since this step also runs with `pull-requests: write` privileges.
Existing Code
        uses: marocchino/sticky-pull-request-comment@v3
maintainability
low
L13-L16
No `concurrency` group is defined while the workflow triggers on `synchronize`. Rapid successive pushes to a PR will spawn redundant lint runs that race on the commit status and sticky comment, producing flapping results and wasted runner minutes. Add a concurrency group keyed on the PR number with `cancel-in-progress: true` so superseded runs are cancelled.
Existing Code
on:
  pull_request:
    types: [opened, edited, synchronize, reopened]
    branches: [main]
Suggested Change
concurrency:
  group: pr-lint-${{ github.event.pull_request.number }}
  cancel-in-progress: true

on:
  pull_request:
    types: [opened, edited, synchronize, reopened]
    branches: [main]
bug
medium
L18-L21
For `pull_request` events originating from forks, GitHub injects a read-only `GITHUB_TOKEN` regardless of these declared permissions. As a result, the commit status written by `action-semantic-pull-request` (needs `statuses: write`) and the `Post lint result` step (needs `pull-requests: write`) will fail with 403 for external contributors. If fork PRs are expected, switch the trigger to `pull_request_target` (safe here because no PR head code is checked out or executed) or make the comment step tolerant with `continue-on-error: true`.
Existing Code
permissions:
  contents: read
  pull-requests: write
  statuses: write
bug
critical
L91-L93
This changelog rewrite is destructive: `out` is built from the new [Unreleased] placeholder + `t[:m.start()]` (the header prefix) + the newly generated release section, but `t[m.end():]` — everything after the old [Unreleased] block, i.e. all previously released changelog entries — is never appended. The file is overwritten with only the header and the new version's notes, permanently deleting release history. Append the tail to the output.
Existing Code
            released = f'## [{ver}] - {date}' + m.group(1)
            out = '## [Unreleased]\n\n### Added\n\n_No unreleased changes yet._\n\n' + t[:m.start()] + released
            p.write_text(out, encoding='utf-8')
Suggested Change
            out = t[:m.start()] + '## [Unreleased]\n\n### Added\n\n_No unreleased changes yet._\n\n' + released + t[m.end():]
            p.write_text(out, encoding='utf-8')
bug
high
L97-L99
On a tag push, `actions/checkout` leaves the workspace on a detached HEAD, so there is no local `main` branch and `git push origin main` fails with "src refspec main does not match any". The trailing `|| echo "changelog push skipped"` silently swallows that failure, so the changelog commit is dropped and never reaches the default branch (and the same step is also unreliable on `workflow_dispatch` runs from a branch). Push the current commit explicitly with `HEAD:main` (or fetch the default branch with `fetch-depth: 0` and check it out), and don't hide the failure.
Existing Code
            git add CHANGELOG.md
            git commit -m "chore(release): changelog for ${VER}" || true
            git push origin main || echo "changelog push skipped"
Suggested Change
            git add CHANGELOG.md
            git commit -m "chore(release): changelog for ${VER}" || true
            git push origin HEAD:main
security
high
L17-L18
Top-level `permissions: contents: write` grants repository write access to every job in the workflow, including `build` (only needs read access to check out) and `publish-packages` (which writes to other repos via the cross-repo `RELEASE_PAT`, not the repo token). Use least privilege: set top-level `permissions: contents: read` and give only the `release` job `permissions: contents: write` so a compromised build or manifest-bump step cannot push to this repository.
Existing Code
permissions:
  contents: write
Suggested Change
permissions:
  contents: read
security
medium
L39-L40
Third-party actions are referenced by mutable refs: `dtolnay/rust-toolchain@stable` is a moving channel and `Swatinem/rust-cache@v2` is a retaggable tag. A retagged or hijacked version can inject arbitrary code into the build job, which runs with the repository token. Pin both to full commit SHAs (verified via the project's release history).
Existing Code
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
Suggested Change
      - uses: dtolnay/rust-toolchain@<full-commit-sha>
      - uses: Swatinem/rust-cache@<full-commit-sha>
security
medium
L113
`softprops/action-gh-release@v3` is a mutable tag rather than a pinned commit SHA, so the action that receives `GITHUB_TOKEN` with write permission could be silently swapped. Pin it to the full commit SHA of the reviewed release.
Existing Code
        uses: softprops/action-gh-release@v3
Suggested Change
        uses: softprops/action-gh-release@<full-commit-sha>
bug
high
L12-L15
`workflow_dispatch` can be triggered from a branch (e.g. the default branch), in which case `GITHUB_REF_NAME` is `main` and `VER` is not a semantic version. The changelog step would then generate a malformed `## [main] - ...` entry and `publish-packages` would produce invalid asset URLs like `.../download/vmain/webrain-windows.exe`. Restrict the trigger to version tags only, or add a guard step that validates `GITHUB_REF_NAME` matches `^v[0-9]+\.[0-9]+\.[0-9]+` and fails otherwise.
Existing Code
on:
  push:
    tags: ["v*"]
  workflow_dispatch:
Suggested Change
on:
  push:
    tags: ["v*"]
other
low
L10
No `timeout-minutes` or `concurrency` is defined anywhere in the workflow. A hung step (network flake, `gh api` retry) can consume runner time indefinitely, and two runs for the same tag can race on changelog commits and release assets. Add a job-level `timeout-minutes` and a `concurrency` group keyed on the ref.
Existing Code
name: Release
Suggested Change
name: Release

concurrency:
  group: release-${{ github.ref }}
  cancel-in-progress: false
bug
high
L56-L57
This comment step is unreachable dead code. Its `if:` condition is exactly the same as the 'Fail if changelog missing' step's condition (PR + non-dependabot + changelog_changed == '0' + source_changed > '0'). Whenever that condition is true, the fail step above exits 1, and since this step has no explicit status function (implicit `success()`), it is always skipped exactly when it should fire. Reorder the steps (run the comment first) or add `if: always() && ...` so the PR comment can actually be posted.
Existing Code
      - name: Comment if changelog missing
        if: github.event_name == 'pull_request' && github.actor != 'dependabot[bot]' && steps.changelog_check.outputs.changelog_changed == '0' && steps.changelog_check.outputs.source_changed > '0'
Suggested Change
      - name: Comment if changelog missing
        if: always() && github.event_name == 'pull_request' && github.actor != 'dependabot[bot]' && steps.changelog_check.outputs.changelog_changed == '0' && steps.changelog_check.outputs.source_changed > '0'
security
medium
L58
Third-party action referenced by a mutable tag (`@v3`). Tags can be force-moved/compromised, enabling supply-chain attacks. Pin it to a full commit SHA (e.g., `marocchino/sticky-pull-request-comment@<full-40-char-sha>`), and keep the tag as a comment for readability.
Existing Code
        uses: marocchino/sticky-pull-request-comment@v3
bug
medium
L43-L44
The `|| true` at the end of each pipeline silently swallows failures from `git diff` itself (e.g., an invalid `RANGE`). In that case grep reads empty stdin and outputs `0`, so both outputs become `0`, the `source_changed > '0'` guard is false, and the enforcement passes without ever checking anything. Compute the diff once, fail explicitly on diff errors, and only mask grep's legitimate 'no match' exit code (1).
Existing Code
          CHANGED=$(git diff --name-only $RANGE | grep -c "CHANGELOG.md" || true)
          echo "changelog_changed=$CHANGED" >> $GITHUB_OUTPUT
Suggested Change
          DIFF_FILES=$(git diff --name-only $RANGE) || { echo "::error::git diff failed for range $RANGE"; exit 1; }
          CHANGED=$(printf '%s\n' "$DIFF_FILES" | grep -c "CHANGELOG.md" || true)
          echo "changelog_changed=$CHANGED" >> $GITHUB_OUTPUT
bug
medium
L50-L51
The `dependabot[bot]` exemption only applies to the pull_request path. This workflow also runs on `push` to main (after a merge), where `github.actor` is the merger, not dependabot. Since `Cargo.toml`/`Cargo.lock` match the source regex, a merged dependabot dependency bump without a CHANGELOG entry will fail CI on main and block the branch. Either apply the exemption to the push path too (e.g., skip when the push contains only dependabot commits) or don't count Cargo.lock-only changes as requiring a changelog.
Existing Code
      - name: Fail if changelog missing
        if: github.actor != 'dependabot[bot]' && steps.changelog_check.outputs.changelog_changed == '0' && steps.changelog_check.outputs.source_changed > '0'
security
low
L36-L37
`${{ github.base_ref }}` and `${{ github.event.before }}` are interpolated directly into the `run:` script. Git ref names may contain shell metacharacters (`$`, backticks, `;`, `&&`, etc.), so a malicious ref could inject commands. (Today `base_ref` is effectively always `main` and `before` is a SHA, so exploitability is low, but the pattern is fragile.) Pass these values via an `env:` block and reference `$BASE_REF`/`$BEFORE_SHA` in the script.
Existing Code
          if [ "${{ github.base_ref }}" != "" ]; then
            RANGE="origin/${{ github.base_ref }}...HEAD"
Suggested Change
        env:
          BASE_REF: ${{ github.base_ref }}
          BEFORE_SHA: ${{ github.event.before }}
        run: |
          if [ "$BASE_REF" != "" ]; then
            RANGE="origin/$BASE_REF...HEAD"
other
low
L24
Job has no `timeout-minutes`, so a hung checkout/diff could consume a runner indefinitely. Add an explicit timeout (e.g., `timeout-minutes: 10`).
Existing Code
    runs-on: ubuntu-latest
Suggested Change
    runs-on: ubuntu-latest
    timeout-minutes: 10
other
low
L10-L15
No `concurrency` group is defined, so pushes to main and multiple PR synchronizations can spawn redundant, overlapping enforcement runs. Add a `concurrency` block keyed on the ref with `cancel-in-progress: true` to skip stale runs.
Existing Code
on:
  pull_request:
    types: [opened, synchronize]
    branches: [main]
  push:
    branches: [main]
Suggested Change
concurrency:
  group: changelog-enforce-${{ github.ref }}
  cancel-in-progress: true

on:
  pull_request:
    types: [opened, synchronize]
    branches: [main]
  push:
    branches: [main]
maintainability
medium
L8
The pattern `vision/` is not root-anchored, so it matches any directory named `vision` at any depth in the repo. If only a top-level build output was intended, this could silently exclude source or data directories elsewhere in the tree. Consider anchoring it to `/vision/` (or use `**/vision/`) so the ignore scope matches the intent.
Existing Code
vision/
Suggested Change
/vision/
maintainability
low
L3
`/build_err.txt` is listed twice (also at line 3). The duplicate adds no value and makes future edits to the ignore rules error-prone. Remove this redundant entry.
Existing Code
/build_err.txt
security
medium
L15
Both stages use floating tags (`rust:alpine`, `alpine:latest`), so builds are not reproducible — an upstream base-image change can silently swap the toolchain/glibc behavior or break the build, and the `apk add chromium` line is likewise unpinned. For a supply-chain-conscious server image, pin each base to a specific version plus digest (e.g. `alpine:3.21@sha256:...`, obtainable via `docker buildx imagetools inspect`) so rebuilds are deterministic and auditable.
Existing Code
FROM rust:alpine AS builder
Suggested Change
FROM rust:alpine@sha256:<pinned-digest> AS builder
security
high
L24-L26
The runtime stage has no `USER` directive, so webrain and the Chromium it spawns run as root. Two concrete consequences: (1) Linux Chromium refuses to start as root without `--no-sandbox`, and `launch_chrome_opt` in webrain-core/src/launch.rs does not pass that flag — browser launches will fail in this image (or you must disable the sandbox, defeating container isolation); (2) any RCE through Chromium or the MCP endpoint yields full root in the container. Add a non-root user with a writable HOME (profiles_dir()/vault derive from $HOME). Note: docker/docker-compose.yml mounts /root/.config and /root/.cache, so those volume paths must be adjusted alongside this change.
Existing Code
COPY --from=builder /app/target/release/webrain /usr/local/bin/webrain
EXPOSE 9223
ENTRYPOINT ["webrain"]
Suggested Change
COPY --from=builder /app/target/release/webrain /usr/local/bin/webrain
RUN addgroup -S webrain && adduser -S -G webrain -h /home/webrain webrain
ENV HOME=/home/webrain
USER webrain
EXPOSE 9223
ENTRYPOINT ["webrain"]
security
medium
L18
`COPY . .` ships the entire repository (minus the current .dockerignore exclusions) into the image layers — any key/config/credential file not on the exclusion list ends up baked into the image and is visible to anyone who pulls it, and any repo change invalidates this layer. Prefer copying only what the build needs (Cargo.toml, Cargo.lock, and the workspace crate sources), and tighten .dockerignore to exclude dotfiles/secret-like files so the build context stays minimal.
Existing Code
COPY . .
performance
medium
L19
A plain `cargo build` re-resolves and recompiles the whole dependency tree on every build; combined with `COPY . .` invalidating the layer on any repo change, every CI/rebuild pays the full compile cost. Since the file already opts into BuildKit via `# syntax=docker/dockerfile:1`, use cache mounts for the registry and target dirs (or the copy-Cargo-files-first / build-deps / copy-source two-step pattern) so dependencies are reused across builds.
Existing Code
RUN cargo build --release --bin webrain
Suggested Change
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin webrain
bug
medium
The exclusion list is incomplete: `docs/README.md` and `docs/images/landing/VEOPROMPT.md` exist under the docs directory, are NOT referenced in the docs.json navigation (only the .mdx pages are), and are not matched by any pattern here. Per the intent stated in the header comment ("the site is the pages in docs.json nav"), these files can still be processed/published by Mintlify. If they are legacy/working files (VEOPROMPT.md in particular looks like an asset draft), add them here to avoid accidental publication.
Existing Code
SERP_CLAIMS_REVIEW.md
Suggested Change
SERP_CLAIMS_REVIEW.md
README.md
images/landing/VEOPROMPT.md
maintainability
low
L9
This glob matches no file currently in the repo — searching for "Mintlify Documentation" and for filenames containing "Mintlify" returns nothing, so the entry is stale. It is also over-broad: any future user-facing doc whose filename starts with "Mintlify Documentation" would be silently excluded from the site. Remove the entry or pin the exact filename(s) that were intended so the ignore set stays auditable.
Existing Code
Mintlify Documentation*.md
maintainability
low
L8
No `.mmd` file exists in the repo anymore (file search for `*.mmd` returns nothing), so this entry appears to be stale — likely the diagram was removed after the audit mentioned in CHANGELOG.md. Keeping dead entries makes it hard to tell which patterns are actually protecting files. Drop this line unless the file is expected to be restored.
Existing Code
arch_diagram.mmd
maintainability
low
L2
The comment documents intent but provides no enforcement mechanism, so new legacy/working files dropped into docs/ can silently become publishable (that is exactly how README.md and images/landing/VEOPROMPT.md slipped through). Consider adding a CI check that asserts every `.md`/`.mdx` under docs/ is either referenced in docs.json navigation or listed in .mintignore, so internal docs can never be published by accident.
Existing Code
# (Legacy/internal docs + working files — the site is the pages in docs.json nav.)
security
medium
L12
No patterns exclude local secrets. The builder stage in docker/Dockerfile runs `COPY . .`, so any `.env`, `.env.local`, key/credential file at the context root (or nested, since these glob patterns match at any depth) would be shipped to the Docker daemon and baked into the builder image layers / build cache — even though the runtime stage only copies the compiled binary. Add exclusions for `.env*` and common credential/key file patterns to keep secrets out of the build context.
Existing Code
.github
Suggested Change
.github
.env*
*.pem
*.key
*.p12
id_rsa*
credentials*
maintainability
low
L2
Docker matches patterns without a leading `/` at any directory depth (gitignore-style semantics), not just the context root. So `target` already excludes nested Cargo build dirs (e.g. `webrain-core/target`), `*.md` excludes markdown in subdirectories, and `.git` excludes nested VCS dirs — there is no gap for nested build outputs. This behavior is desirable for a Rust workspace; if a root-only exclusion is ever intended, prefix the pattern with `/`.
Existing Code
target