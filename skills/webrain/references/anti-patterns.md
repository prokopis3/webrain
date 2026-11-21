# Anti-Patterns (what NOT to do)

DO NOT:
- start protected workflows statelessly (anonymous → blocked → fresh browser → retry)
- discard a working profile
- discard a working session
- switch to a fresh browser after a challenge (you lose the identity/session that would get you through)
- treat challenge detection as successful scraping (a challenge page is not content)
- assume CAPTCHA handling exists without checking runtime capability
- reintroduce a Python/stealth sidecar (challenge handling is native)
- restore the obsolete stealth architecture
- claim a bypass succeeded without verification (verify the target content)
- extract challenge/login/consent pages as target content
- create sequential loops when `webrain_batch`/`webrain_crawl` exist
- dump raw HTML to the model when `observe`/`extract` give text/structure cheaper
- wait 30 s for `visible` on an SPA list instead of `attached` + 6 s on the card
  selector (measured: 159 s → 10 s for 27 URLs, same rows)
- anchor a wait on a shell element that exists before the cards (`#ProductsClass`:
  0.2-2.4 s, `found: 0` everywhere) — anchor on the repeated card instead
- blame `concurrency` for a slow batch before reading `ms_total` vs the real page
  time: wait timeouts look exactly like a slow site
