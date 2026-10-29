# Other ideas explored (and why they were set aside)

The conversation used questions first, then ideas. For reference:

- **TTRPG campaign chronicle site** (session recaps, NPCs, quotes, read-only for friends, installable PWA for offline): fits Proa's multi-target rendering, but was based on a wrong assumption that the project had to be TTRPG-related.
- **Group leaderboard and ledger / game night planner / group archive:** reasonable small tools for a friend group; not exciting to the author.
- **Game dev tools** (content database + codex site, playtest telemetry dashboard, devlog and builds page, in-house GPUI editors): not useful to the author's current workflow.
- **Personal trackers** (eating out log, gas fill-ups, game library and playtime): judged "kind of useless".
- **What developers are building (GitHub trending, Ask HN):** AI agent tooling dominates. The author did not want a full AI harness because of the baggage. That led to ideas around the harness: an agent session dashboard, a docs site for humans and agents, a curated games catalog, a small Bevy game with a Proa leaderboard.
- **The chosen direction:** a plan-driven, ComfyUI-like orchestrator that drives existing agents over ACP, snapshots with jj, and sits above Delta and other harnesses (see `01-concept.md`).
