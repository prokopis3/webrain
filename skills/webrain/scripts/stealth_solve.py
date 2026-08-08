#!/usr/bin/env python3
"""stealth_solve.py — real-Chrome stealth sidecar for webrain (self-contained copy).

Launches a real, stealth-patched Chrome with a CDP port, solves a Cloudflare
challenge (managed JS challenge / Turnstile), logs in with the demo credentials,
and exports cookies. The Chrome stays up (default) so the webrain MCP tools can
attach to the SAME authenticated session via CDP (CDP_URL=http://127.0.0.1:<port>).

  python stealth_solve.py <url> [--creds user:pass] [--out cookies.json]
      [--cdp-port 9222] [--wait 60] [--headed] [--exit-after]

ponytail: launch Chrome ourselves (subprocess) then connect_over_cdp so cookies
land in the DEFAULT profile context webrain attaches to — playwright's own
launch would hide them in an incognito context.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import time

# Real Chrome wins (patchright's #1 best practice — CfT/Chromium is more
# fingerprintable); WEBRAIN_CHROME overrides; CfT cache is the last fallback.
def chrome_binary() -> str:
    cands = [
        os.environ.get("WEBRAIN_CHROME", ""),
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ]
    for c in cands:
        if c and os.path.exists(c):
            return c
    # CfT cache fallback (webrain install chrome).
    for base in (os.environ.get("LOCALAPPDATA", ""), os.path.expanduser("~")):
        p = os.path.join(base, "AppData", "Local", "webrain", "browsers")
        if os.path.isdir(p):
            for d in sorted(os.listdir(p), reverse=True):
                cand = os.path.join(p, d, "chrome-win64", "chrome.exe")
                if os.path.isfile(cand):
                    return cand
    return "chrome"


CHROME = chrome_binary()


def challenge_pending(page) -> bool:
    # title-only: the interstitial keeps "Just a moment..." until solved. The
    # __cf_chl_tk query param survives the solve, so URL-based checks lie.
    title = (page.title() or "").lower()
    return ("just a moment" in title) or ("performing security verification" in title)


def turnstile_token(page) -> str:
    """Value of the hidden cf-turnstile-response input ('' = not solved yet)."""
    try:
        return page.eval_on_selector(
            '[name="cf-turnstile-response"], #cf-chl-widget_response',
            "el => el.value || ''",
        )
    except Exception:
        return ""


def click_turnstile(page) -> bool:
    """Click the interactive Turnstile checkbox iframe if it rendered (cf-turnstile
    variant: no 'Just a moment' interstitial, a clickable checkbox instead)."""
    try:
        # The widget container is #cf-turnstile / .cf-turnstile; the clickable
        # checkbox lives in the cross-origin iframe it hosts.
        frames = page.frames
        for f in frames:
            if "challenges.cloudflare.com" in (f.url or ""):
                try:
                    cb = f.locator("input[type=checkbox], .ctp-checkbox-label").first
                    if cb.count():
                        cb.click(timeout=3000)
                        return True
                except Exception:
                    pass
        # Fallback: click the widget container area.
        w = page.locator("#cf-turnstile, .cf-turnstile").first
        if w.count():
            w.click(timeout=3000)
            return True
    except Exception:
        pass
    return False


# Session-ish cookies: presence of any after a submit implies the account is
# logged in (Instagram sessionid, Google SID, Facebook c_user, ...).
SESSION_COOKIES = {"sessionid", "session", "SID", "SSID", "APISID", "SAPISID",
                   "c_user", "datr", "dpr", "mid", "ig_did", "auth_token"}


def logged_in(ctx) -> bool:
    return any(c["name"] in SESSION_COOKIES for c in ctx.cookies())


def submit_login(page, user, pwd):
    """Fill the visible login form and submit it. Returns True if a submit fired."""
    fields = page.locator('input[name="email"], input[name="username"], input[name="login"], input[type="email"]').first
    if not fields.count():
        return False
    pw = page.locator('input[name="password"], input[type="password"]').first
    if not pw.count():
        return False
    fields.fill(user)
    pw.fill(pwd)
    # Most sites: a real <button type=submit>. Instagram: a <div role=button>.
    # Click by accessible name first, fall back to Enter on the password field.
    for label in ("Log in", "Sign in", "Continue", "Submit", "Log In", "Sign In"):
        try:
            b = page.get_by_role("button", name=label, exact=True).first
            if b.count():
                b.click(timeout=5000)
                return True
        except Exception:
            continue
    try:
        pw.press("Enter")
        return True
    except Exception:
        return False


def twofa_signal(page) -> bool:
    """Detect a 2FA / approval / device-verification gate that needs the human."""
    url = (page.url or "").lower()
    if any(k in url for k in ("challenge", "two_factor", "checkpoint", "verify",
                              "onetap", "login_required", "sms", "otp", "2fa")):
        return True
    try:
        if page.locator('input[autocomplete="one-time-code"], input[name="code"], '
                        'input[name="otp"], input[inputmode="numeric"][maxlength="6"]').first.count():
            return True
    except Exception:
        pass
    try:
        body = (page.inner_text("body") or "")[:4000].lower()
        if any(k in body for k in ("enter the code from your authenticator app",
                                   "enter your verification code", "approve this device",
                                   "confirm it's you", "check your phone",
                                   "we've sent a code", "enter the 6-digit code")):
            return True
    except Exception:
        pass
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("url")
    ap.add_argument("--creds", default="admin@example.com:password")
    ap.add_argument("--out", default=None, help="write cookies JSON here")
    ap.add_argument("--cdp-port", type=int, default=9222)
    ap.add_argument("--wait", type=int, default=60, help="max seconds to wait out the challenge")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--exit-after", action="store_true", help="export cookies then close Chrome")
    ap.add_argument("--profile", default=None, help="persistent Chrome profile dir (per-account cookies); default: throwaway temp")
    args = ap.parse_args()

    user, _, pwd = args.creds.partition(":")
    if args.profile:
        os.makedirs(args.profile, exist_ok=True)
        profile = args.profile
    else:
        profile = tempfile.mkdtemp(prefix="stealth_")

    chrome_args = [
        CHROME,
        f"--remote-debugging-port={args.cdp_port}",
        "--remote-debugging-address=127.0.0.1",
        f"--user-data-dir={profile}",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-blink-features=AutomationControlled",
        "--disable-features=AutomationControlled",
    ]
    if not args.headed:
        chrome_args.append("--headless=new")
    chrome_args.append("about:blank")

    proc = subprocess.Popen(chrome_args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    # patchright = patched Playwright driver (Runtime.enable leak, Console.enable,
    # command-flag leaks, sourceURL — the CDP-level things Cloudflare checks) +
    # real Chrome. Falls back to stock playwright + undetected_playwright JS
    # evasions when patchright isn't installed.
    try:
        from patchright.sync_api import sync_playwright
        use_patchright = True
    except ImportError:
        from playwright.sync_api import sync_playwright
        from undetected_playwright import stealth_sync
        use_patchright = False

    try:
        with sync_playwright() as p:
            browser = None
            for _ in range(40):
                try:
                    browser = p.chromium.connect_over_cdp(f"http://127.0.0.1:{args.cdp_port}")
                    break
                except Exception:
                    time.sleep(0.5)
            if browser is None:
                print("could not attach to Chrome CDP", flush=True)
                return 2

            ctx = browser.contexts[0]  # DEFAULT profile context webrain will also see
            if not use_patchright:
                ctx = stealth_sync(ctx)
            page = ctx.new_page()

            page.goto(args.url, wait_until="domcontentloaded", timeout=45000)
            print(f"initial title={page.title()!r} url={page.url}", flush=True)

            # wait out the Cloudflare interstitial — let ITS js solve it; reloading
            # here restarts the proof each time (__cf_chl_rt_tk rotates per reload).
            t0 = time.time()
            last = time.time()
            while time.time() - t0 < args.wait and challenge_pending(page):
                time.sleep(3)
                if time.time() - last > 15 and challenge_pending(page):
                    last = time.time()
                    try:
                        page.reload(wait_until="domcontentloaded", timeout=25000)
                    except Exception:
                        pass
            cleared = not challenge_pending(page)
            print(f"challenge_cleared={cleared} title={page.title()!r} url={page.url}", flush=True)

            # cf-turnstile variant: no "Just a moment" interstitial — a plain
            # login form with a Turnstile checkbox. Click it and wait for the
            # hidden token to populate before submitting (empty token = 403).
            if cleared and page.locator('#cf-turnstile, .cf-turnstile, [name="cf-turnstile-response"]').first.count():
                if turnstile_token(page) == "":
                    click_turnstile(page)
                    t0 = time.time()
                    while turnstile_token(page) == "" and time.time() - t0 < 30:
                        time.sleep(2)
                print(f"turnstile_token={'set' if turnstile_token(page) else 'EMPTY'}", flush=True)

            # ponytail: only auto-login when real creds were given — the human-types
            # path (empty creds, per the credentials skill) must leave the form alone.
            if cleared and user and pwd:
                submitted = submit_login(page, user, pwd)
                print(f"post_login url={page.url} title={page.title()!r} submitted={submitted}", flush=True)
                if submitted:
                    # Wait for the session to land. If a 2FA/approval gate shows up,
                    # park and let the HUMAN finish it in the headed browser.
                    gate = twofa_signal(page)
                    if gate:
                        print("2fa: waiting for user in the browser (approve or enter code)", flush=True)
                    t0 = time.time()
                    while not logged_in(ctx):
                        if not gate and time.time() - t0 > 15:
                            break
                        time.sleep(3)
                        if twofa_signal(page) and not gate:
                            gate = True
                            print("2fa: waiting for user in the browser (approve or enter code)", flush=True)
                print(f"logged_in={logged_in(ctx)}", flush=True)
            else:
                print("no_auto_login (human types in browser)", flush=True)

            cookies = ctx.cookies()
            names = {c["name"] for c in cookies}
            if args.out:
                with open(args.out, "w") as f:
                    json.dump(cookies, f, indent=2)
                print(f"cookies -> {args.out}", flush=True)
            print(f"cookies={len(cookies)} cf_clearance={'cf_clearance' in names} "
                  f"cdp=http://127.0.0.1:{args.cdp_port}/devtools/browser", flush=True)

            if args.exit_after:
                return 0 if (cleared and "cf_clearance" in names) else 3

            print("keeping Chrome alive for webrain; Ctrl-C to stop", flush=True)
            while True:
                time.sleep(60)
    finally:
        if args.exit_after:
            proc.terminate()


if __name__ == "__main__":
    sys.exit(main())
