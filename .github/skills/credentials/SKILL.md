---
name: credentials
description: 'Secure credential handoff for any MCP-server task. Use automatically when a task prompt mentions login, credentials, password, sign in, log in, auth, or token (e.g. "login to instagram with credentials then scrape posts"). The rule: secrets NEVER pass through the model — have the user type them into a terminal or headed browser, set env vars, or store them in the local vault; reference them without ever seeing, printing, or logging the values.'
argument-hint: 'The task that needs credentials (e.g. login to instagram and scrape 20 posts)'
---

# /credentials — never see the secret

Any task mentioning login / credentials / password / sign-in / token fires
this. **Secrets never pass through the model.** Anything asked in chat
round-trips through the LLM and lands in session logs — so never ask in chat.

## Protocol (stop, pick ONE channel)

1. Detect login/creds intent in the task. Pause the happy path.
2. Tell the user which channel to use. **Never request the value in chat**
   (no `vscode_askQuestions`, no "please provide your password").
3. Use the value, never print it; redact it if it ever shows up in args/output.

### Channel A — user types into the real browser (preferred, `mcp_webrain-*`)
`webrain launch <service> <profile> <url>` — real Chrome opens headed on a
persistent profile, the **user** types the creds into the site's login window,
the session is saved to the profile, and webrain re-attaches to the same CDP
session. You get the authenticated session, never the password.

### Channel B — env vars (no browser, or `--creds`)
User sets the secret in the terminal (never in chat):
```powershell
$env:WEBRAIN_USER = "..."                        # typed by user
$env:WEBRAIN_PASS = Read-Host "password"         # typed by user
```
Then `webrain login <service> <profile>` (or `webrain_session(op=login)`) reads
them by name — never inline the values, never echo them.

### Channel C — local vault (fully automatic, preferred for repeat logins)
One-time enroll, then the LLM logs in with zero user typing:
```bash
webrain vault set <service> <profile> --username <user>   # hidden prompt, once
```
- `webrain_profiles` → returns ids/usernames only — never secrets.
- `webrain_login(profile=…, url=…, user_sel=…, pass_sel=…, submit_sel=…)`
  → the server decrypts in-process and fills the browser via CDP; the reply is
  status-only. The value never leaves the server process.
- Storage: `~/.config/webrain/vault.json` (AES-256-GCM) + `vault.key` (0600).
  Two files — copy both to any Windows/macOS/Linux/Docker box, no OS daemon.

## Rules

- Never call any chat/ask tool for a secret value; the answer is logged.
- Never paste a credential into a command, tool arg, or file visible in the conversation.
- Never pass a secret as a tool argument — pass a vault reference (profile id) and let the server resolve it.
- Redact immediately if a credential appears in output.
- Same rule for every MCP server (webrain, codebase-memo, …), not just webrain.
