#!/usr/bin/env python3
"""webrain skill preflight — is the MCP server up? which browser is on CDP?

Modes:
  preflight.py            JSON: {mcp_up, mcp_url, cdp:[{port,browser,kind}], recommend}
  preflight.py --check    silent: exit 0 if mcp + a browser are up, else 2

ponytail: stdlib only (urllib). kind = real-chrome | obscura | unknown from the
Browser version string; recommend = the best backend to use.
"""
import json
import sys
import urllib.request

MCP_URL = "http://127.0.0.1:9223/mcp"
CDP_PORTS = [9222, 9224]


def _post(url: str, payload: dict):
    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=4) as r:
        return r.status


def _cdp_version(port: int) -> str:
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=3) as r:
            return str(json.load(r).get("Browser", "") or "")
    except Exception:
        return ""


def status() -> dict:
    mcp_up = False
    try:
        mcp_up = _post(MCP_URL, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}) == 200
    except Exception:
        mcp_up = False

    cdp = []
    for p in CDP_PORTS:
        b = _cdp_version(p)
        if b:
            low = b.lower()
            kind = "real-chrome" if ("chrome" in low and "obscura" not in low) else ("obscura" if "obscura" in low else "unknown")
            cdp.append({"port": p, "browser": b, "kind": kind})

    recommend = (
        "real-chrome" if any(c["kind"] == "real-chrome" for c in cdp)
        else ("obscura" if cdp else "none")
    )
    return {"mcp_up": mcp_up, "mcp_url": MCP_URL, "cdp": cdp, "recommend": recommend}


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--check":
        s = status()
        sys.exit(0 if (s["mcp_up"] and s["cdp"]) else 2)
    print(json.dumps(status(), indent=2))
