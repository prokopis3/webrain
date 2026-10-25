---
name: product-market-research
version: "1.0.0"
description: Compare products across shops/e-shops on price AND on the spec that actually matters (protein, calories, active ingredient, capacity, warranty) — normalise units, match the same SKU across retailers, and rank by value. Use for "find the best X in <market>", grocery/food shopping, price monitoring, spec comparison, alternatives and cheaper substitutes.
argument-hint: "<what to buy / compare> [in <market or shops>] [priority: spec | price]"
allowed-tools: Bash, Read, Write
---

# /product-market-research

Read `web-research` first — same loop (**discover → batch → persist → verify →
report**), plus the three things that only matter when the data is a product:
**per-unit normalisation**, **spec extraction**, **cross-shop SKU matching**.

## The three traps

### Trap 1 — "per serving" columns
Nutrition/spec tables frequently print the **per-serving column first**
(25 g, 30 g, 20 g) and per-100 g second. Parsing the first number gives
graviera **6 g** protein instead of **25 g** — a 4× error that silently
reorders your whole ranking.

Detect the column before parsing:
```js
const hdr = text.slice(0, text.indexOf('Ενέργεια'));   // or 'Energy'/'Calories'
const col = hdr.indexOf('100') < (hdr.search(/serving|μερίδα|portion/i) + 1e9) ? 0 : 1;
```
Rule: **find "per 100 g" and take that column**, then assert the value is
plausible for the category (cheese 20–30 g/100 g, yogurt 4–10, canned fish 15–25).
An implausible number is a parse bug or a retailer data error — investigate, and
say which in the report.

### Trap 2 — comparing different pack sizes
Price alone ranks the 1 kg tub last and the 200 g cup first. Always compute:
```
unit_price  = price / pack_size          (already printed on most e-shops)
value_metric = price / (quantity_of_the_thing_you_care_about)
```
For "protein value" the readable metric is **€ per 10 g of protein**:
`price / (protein_per_100g × pack_g / 1000)`. Generalise the denominator to
whatever the user prioritised — g of active ingredient, litres, washes, m².

### Trap 3 — assuming the same name is the same product
Match on **brand + exact size + variant**, never on title similarity. Then report
the delta and *which shop wins*:
`Δ = (A − B) / B`. State the direction in the header (`+10%` = A is dearer).

## Workflow

1. **Define the criterion first.** Ask or infer: is nutrition/spec the priority,
   or price? That decides the value metric and the ranking order. When the user
   says "quality first, then value", produce **two rankings** and a combined
   verdict — do not silently blend them.
2. **Pick canonical items, not endless listings.** Choose the 10–15 SKUs or
   categories that carry the decision (staple + premium + own-brand). Searching
   each shop for the same canonical term makes the cross-shop join possible;
   browsing each shop's taxonomy does not.
3. **Per-shop discovery, then per-shop batch.** Shops differ in kind, so probe
   each host once with `webrain_scrape(url, max_chars=2000)` before reaching for
   a browser:
   - **server-rendered** (`needs_js:false`, rows visible in `text`) → extract the
     whole price list browserlessly with
     `webrain_scrape(base_selector=..., fields=...)`. This fetches the full body
     and parses it in-page-less: measured 266–415 ms per shop versus 800–2000 ms
     per browser tab, and no tab pool to exhaust. **Most grocery e-shops are in
     this bucket — do not pay the browser tax for them.**
   - **SPA/Stencil/React** → `webrain_batch(op=eval, wait_selectors=[...])` with a
     card selector per host, `wait_timeout_secs` sized for the slowest.
   - **store-gated** (price only after postcode/store/delivery slot) → the page
     loads but the price never renders. That is a *human step*, not a parsing bug.
4. **Harvest listing cards with their embedded JSON.** Most e-commerce ships the
   product payload in an attribute — `data-plugin-*`, `data-ga-*`, `data-item`,
   `dataset` — which gives name/brand/price/SKU without regexing formatted text.
   Fall back to `innerText` only when there is no payload attribute.
5. **Fetch the spec page only for the shortlist.** Listing cards rarely carry the
   nutrition/spec table; product pages do. Two-stage: cheap cards for the market
   map, expensive product pages for ~30 shortlisted items.
6. **Join prices to specs by URL**, then rank.

## Shops seen in the wild (Greece, Oct 2026)

Useful as a starting map; verify before trusting — sites change.

| Shop | Search URL | Rendering | Price needs store? |
|---|---|---|---|
| Sklavenitis | `/apotelesmata-anazitisis/?Query=<q>` | SSR, cards `div.product` with `data-plugin-analyticsimpressions` JSON | no |
| My Market | `/search?query=<q>` (note: `query`, not `q`) | SSR, cards `article.product--teaser` with `data-google-analytics-item-value` JSON | no |
| Lidl Hellas | `/q/search?q=<q>` | Stencil + **shadow DOM**; `wait_selector="odsc-tile"`, deep-text walk required | online assortment is thin; prices are in-store |
| AB Vassilopoulos | `/el/eshop/<Cat>/<Sub>/<Product>/p/<id>` | React/Next SPA | **yes** — no price without postcode/slot |
| Masoutis | — | Angular SPA, no stable search route | **yes** |

Sklavenitis product pages carry a full **Διατροφική Δήλωση** (energy, fat,
carbs, sugars, protein, salt) **and** the ingredient list — that is what makes
"no added sugar / gluten-free" verifiable instead of guessed.

## Verifying "free-from" claims

A spec value (0 g sugars) is not the same as a claim (no added sugar).
Check the **ingredient list** for the actual additive:

- sugar: `ζάχαρη`, `σιρόπι`, `δεξτρόζη`, `γλυκόζη`, `sugar`, `syrup`, `maltodextrin`
- gluten: `σιτάρι`, `γλουτένη`, `βρώμη`, `κρίθαρι`, `wheat`, `barley`, `rye` —
  and remember "χωρίς γλουτένη" is legally binding **only when declared**.

Report the two separately: *naturally 0 g* vs *no added sugar (label) *.

## Report format that survives scrutiny

```
README.md            TL;DR table + methodology + limitations + date
01-<spec>.md         ranked by the primary criterion
02-value.md          ranked by value metric
03-price-comparison  same SKU across shops, % delta, winner
04-catalog.md        everything collected, grouped
05-coverage.md       what was NOT covered and why (store gates, assortment)
data/*.json          raw + normalised
scripts/             the reproducible aggregation
```

Every row carries its shop and URL. Every excluded item gets a reason (e.g.
"retailer declares 10 g protein for graviera — data error, excluded").

## Never

- invent a price for a store-gated shop — report the gate
- rank on price without normalising the unit
- blend "nutrition first" and "cheapest" into one number without saying so
- present a spec value you did not read from the label
