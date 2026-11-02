# `.github/skills/` — repo-local only

Skills in here serve **this repository**: changelog upkeep, git cleanup, design work.
They are loaded by the editor's agent when working on webrain itself.

The skills that describe **webrain's tool surface** are not here. They live in
[`skills/webrain/`](../../skills/webrain), because that tree is compiled into the
binary with `include_str!` and shipped by `webrain install skills`, then served to any
model through `webrain_guide(topic="skills")`. Edit them there and nowhere else — a
second copy here silently drifts the moment a tool is renamed, which is how the
`credentials` copy came to reference `webrain_login`/`webrain_profiles` long after
both were folded into `webrain_session`.

See the "Skills" section of [`ARCHITECTURE.md`](../../ARCHITECTURE.md) for the full
layout.
