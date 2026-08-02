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
import subprocess
import sys
import tempfile
import time

CHROME = r"C:\Program Files\Google\Chrome\Application\chrome.exe"


def challenge_pending(page) -> bool:
    # title-only: the interstitial keeps "Just a moment..." until solved. The
    # __cf_chl_tk query param survives the solve, so URL-based checks lie.
    title = (page.title() or "").lower()
    return ("just a moment" in title) or ("performing security verification" in title)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("url")
    ap.add_argument("--creds", default="admin@example.com:password")
    ap.add_argument("--out", default=None, help="write cookies JSON here")
    ap.add_argument("--cdp-port", type=int, default=9222)
    ap.add_argument("--wait", type=int, default=60, help="max seconds to wait out the challenge")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--exit-after", action="store_true", help="export cookies then close Chrome")
    args = ap.parse_args()

    user, _, pwd = args.creds.partition(":")
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

    from playwright.sync_api import sync_playwright
    from undetected_playwright import stealth_sync

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

            if cleared:
                email = page.locator('input[name="email"], input#email, input[type="email"]').first
                if email.count():
                    email.fill(user)
                    pw = page.locator('input[name="password"], input#password, input[type="password"]').first
                    if pw.count():
                        pw.fill(pwd)
                    page.locator('button[type="submit"], #submit-button').first.click()
                    page.wait_for_timeout(4000)
                print(f"post_login url={page.url} title={page.title()!r}", flush=True)

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
