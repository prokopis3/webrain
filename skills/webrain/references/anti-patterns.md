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
