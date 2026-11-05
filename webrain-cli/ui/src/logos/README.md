# Provider marks

One SVG per provider id, `viewBox="0 0 24 24"`, a single filled path drawn in
`currentColor` so the tab's own colour rules still apply. `env.svg` is the mark
for a session started against an endpoint no row claims.

Imported with `import.meta.glob("./logos/*.svg", { query: "?raw" })`, so adding
a provider is adding a file — no code change, and no request per tab.

| file | source | licence |
|---|---|---|
| anthropic, gemini, deepseek, openrouter, qwen | [Simple Icons](https://simpleicons.org) | CC0 1.0 |
| minimax, qianfan (Baidu) | [Simple Icons](https://simpleicons.org) | CC0 1.0 |
| openai, groq, fireworks | [LobeHub icons](https://github.com/lobehub/lobe-icons) | MIT |
| grok, kimi, zhipu, hunyuan, doubao, siliconflow | [LobeHub icons](https://github.com/lobehub/lobe-icons) | MIT |

The marks themselves are trademarks of their owners. They identify the endpoint
you are talking to; nothing here implies endorsement.
