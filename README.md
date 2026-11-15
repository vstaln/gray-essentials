<p align="center">
  <img src="assets/gray-logo.svg" alt="gray" width="96">
</p>
<h1 align="center">gray-quality-of-life</h1>
<p align="center">Detect the other agent tools you use, import their setup, and update everything in one go.</p>
<p align="center">
  <a href="https://github.com/vstaln/gray-qol/blob/main/LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="gray plugin" src="https://img.shields.io/badge/gray-plugin-7aa2f7.svg">
  <img alt="rust" src="https://img.shields.io/badge/built%20with-rust-orange.svg">
</p>

A Rust sidecar (lazy spawn, wire 1.1). It implements the plugin side of
`GRAY-QOL-CONTRACT.md` v2: everything that used to be a convenience hard-wired
into gray core now lives here, on top of the generic host API core exposes.

## What it does

- **Detector** (`session_start`, stat-only, well under 20 ms): looks at
  `~/.claude/`, `~/.claude.json`, `~/.codex/`, `~/.config/opencode/`,
  `~/.cursor/`, `~/.windsurf/`, `~/.config/devin/`, `~/.local/share/devin/`
  and `claude`, `codex`, `opencode`, `devin`, `cursor-agent`, `aider` on
  `PATH`. It never reads file contents. When it finds a tool it has not
  mentioned before, it shows one home row through `host/notice`. If several
  tools are new, they share one combined row.
- **`/qol setup [tool]`**: a multi-select checklist, with a preview for each item:
  - rules (`CLAUDE.md` / `AGENTS.md`), appended to `~/.gray/AGENTS.md` as a
    marked section; the diff is shown first
  - skills, symlinked into `~/.gray/skills/`
  - MCP servers, merged into `~/.gray/mcp.json`; env, header and token
    values are masked in the preview
  - the matching subscription plugin, installed via `host/plugin_install`
  - `*_API_KEY` names found in the environment, with a pointer to `/connect`

  Every write is logged in `~/.gray/qol/state.json`. If you decline (an
  empty selection), the tool is never suggested again.
- **`/qol undo [n]`**: reverts the last n imports.
- **`/qol dismiss <tool>`**: stops notices for a tool. `/qol setup <tool>` still works.
- **`/qol scanned`**: lists every path the detector checked and what it found.
- **`/qol first-run`**: the welcome screen, then setup for any detected tools,
  then a `connect_picker` outcome.
- **`/cd [path]`** (alias `/chdir`): change gray's working directory and resume
  the same conversation there. Bare `/cd` goes home; a target outside the
  current git worktree asks first.
- **`/export [md | txt | file]`**: the current session's conversation (every
  message on the active branch, including what came before a compaction) for
  carrying the work into another agent harness. Bare `/export` asks: copy to clipboard or save to a file. `clip` copies markdown
  to the clipboard; `md` / `txt` write `./gray-<session>.md` / `.txt`; a path
  writes there (`.txt` is plain text, anything else markdown). Tool calls and
  results are kept but cut to about 2,000 characters each; thinking is left out.
- **Away recap** (`user_return`, 3 min away): one sentence on what you were
  doing last, as a `Recap` card. Skipped when the transcript has not changed
  since the last one.
- **Prompt suggestions**: after a turn, the likely next message appears as
  ghost text in the composer; Enter on an empty line accepts it.
- **Automatic session titles**: the first answered turn names the session.
- **Issue links**: `#123` in a reply becomes a link to the repo's issue or PR
  (from `git remote get-url origin`).
- **Prompt templates**: `*.md` files in `.gray/prompts/` (project, up to the git
  root) and `~/.gray/prompts/` become slash commands, with YAML
  frontmatter (`description`, `argument-hint`) and `$ARGUMENTS`, `$1`,
  `${@:2}`, `${1:-default}` placeholders. The list is announced with
  `host/commands_changed`; the host has to re-read the plugin manifest to pick
  new templates up.
- **Auto-continue**: after a usage limit, the task resumes on its own when the
  limit resets.
- **Notify on finish** and **break reminder**: terminal notifications when a
  turn ends, and after a long working streak.
- **Session cleanup**: sessions idle past the configured period are deleted,
  at most once a day.
- **Interrupted hint**: a home row when this directory has interrupted
  sessions, cleared with `/qol sessions dismiss`. Press `i` in the `/qol`
  panel to hide it for good (or show it again); the choice is saved and
  running sessions follow it at their next turn.
- **`/settings` rows** for all of the above (away recap, suggestions, auto
  titles, issue links, session cleanup, auto-continue, notify on finish, break
  reminder) — labels and defaults match what core used to ship.
- **`/qol update`**: checks `gray update --check`, `gray plugin update --check
  all`, provider CLIs (npm / brew / pipx / uv / the tool's own updater) and
  `host/sessions`. It asks once through `host/ask`, then runs provider CLIs →
  `gray plugin update all` → `gray update` → `host/restart {resume}`, and
  prints a result for each item. Logs go to `~/.gray/qol/logs/`.

The plugin writes only under `$GRAY_HOME` (default `~/.gray`), and only
after you say yes.

## Install

```sh
cargo build --release
gray plugin install ./target/release/qol
```

## Development

```sh
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

---
Part of the [gray](https://github.com/vstaln/gray) plugin ecosystem —
the open-source AI agent harness. <https://gray.alignment.id>
