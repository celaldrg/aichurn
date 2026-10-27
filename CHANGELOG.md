# Changelog

## v0.1.0 (2026-10-27)

First release.

- `aichurn [path | owner/repo | url]`: replays first-parent history and tracks the birth time of every line
- **Two-week churn**: share of new lines rewritten or deleted within 14 days (`--window` to change)
- **Copy/paste**: share of new lines inside a 6-line block that already existed in the repo
- **Moved code**: share of added lines that were moved within the same commit
- **AI-assisted vs human** split for every metric, from self-declared signatures of 14 agents (Claude Code, Cursor, Copilot, Codex, Aider, Devin, Jules, Gemini, Amp, opencode, Factory, Windsurf, Lovable, v0); merged branches pass their agent to the merge commit
- Per-quarter or per-month table (`--by`), churn sparkline, partial periods marked
- Bot commits and bulk commits (`--max-commit-lines`) are replayed but not counted; lockfiles, docs, data and generated files are excluded (`--all-files` to include)
- Remote repos are cloned shallowly back to `--since` (default 2 years)
- Outputs: terminal, `--json`, `--markdown`, `--card` (SVG report card), `--badge` (SVG badge); `--fail-above` CI gate
- `AICHURN_VERIFY=1` self-check of the replay against the real files
- Rust standard library only; prebuilt binaries for Linux, macOS and Windows
