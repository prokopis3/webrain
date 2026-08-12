# Core Rules (webrain skill)

Highest-priority operational rules. Follow these before anything else.

1. **Browser state matters.** A browser is execution state, not a disposable
   resource. Do not treat it as one.
2. **Profile state matters.** A persistent profile holds cookies, site data, and
   challenge clearances. Reuse it; never discard a working profile.
3. **Session state matters.** A session ties the profile to a live browser.
   Preserve it across navigations; re-attach rather than restart.
4. **Protected navigation starts with real Chrome.** For unknown, authenticated,
   or protected websites: persistent profile → real Chrome → persistent session →
   navigate. Never start an anonymous "blocked → fresh browser → retry" loop.
5. **Preserve browser identity.** Don't switch to a fresh browser after a
   challenge; that discards the identity/session that would let you through.
6. **A challenge page is not successful navigation.** Read the `challenge` field
   on every navigate. Never extract a challenge/login/consent page as target
   content.
7. **CAPTCHA/challenge capability is runtime-dependent.** Never claim a bypass
   the runtime doesn't provide. Detect → classify → invoke supported capability →
   verify.
8. **Challenge handling is native — no Python sidecar.** Never reintroduce an
   external stealth sidecar. Use `webrain_session(op=login)` / `webrain launch` +
   `webrain login` (vault + TOTP); interactive CAPTCHAs need a human in the
   headed browser.
9. **Never report an unverified bypass.** A cleared challenge must be confirmed
   by the target content actually loading.
10. **Verify final content before returning results.** Confirm the target data
    exists (not a block/consent/empty page), then report success.
