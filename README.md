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

## Secrets

A gray plugin that lets an agent use secrets without ever seeing them. The values live in the desktop keyring (libsecret, through `secret-tool`, with the attributes `service=gray-secrets name=NAME`) or, for the current session only, in the plugin's memory.

### How an agent gets a secret

The agent calls the `secret_request` tool with a name and a purpose. Gray shows a masked field inline in the terminal, and the user types the value there. The user alone can press Tab to tick "Save permanently", which stores the value in the keyring; otherwise the plugin keeps it in memory until the session ends. The value travels from gray's UI to the plugin process only, and the tool result the model sees is `{"name": ..., "status": "stored" | "session" | "declined"}`.

When there is no interactive terminal (print mode, gateway, Discord or Telegram sessions), the prompt is refused and the agent is told to ask the user to run `gray secrets set NAME` in a terminal. A value is never collected through a chat channel.

The prompt needs the `host.secret` capability: `gray plugin capabilities extras --all`.

### Using a secret

The agent writes `{{secret:NAME}}` in a bash command. The plugin's tool/before hook rewrites the command so that it runs inside `gray-extras exec --env NAME -- bash -c '...'`, with the placeholder replaced by `"${NAME}"`. The value is therefore never part of the command text, and it exists only in the child's environment. The wrapper replaces every value, its base64 form and its URL-encoded form with `<secret:NAME>` in the child's stdout and stderr, and it returns the child's exit code unchanged. The tool/after hook, which the manifest marks as required, masks the same values in all other tool output, and gray withholds the output if the hook fails.

Bash commands that call `secret-tool` or touch the plugin's own state are refused with an explanation.

Session values reach the wrapper over a 0600 unix socket in `$XDG_RUNTIME_DIR/gray-secrets/`, authorised by a random per-session token. A small reaper process removes the socket and the plugin's ledger when gray exits. If the plugin restarts mid-session, it no longer holds session values, so tool/after withholds output until they are requested again.

Values shorter than 6 characters are not masked, and the plugin warns about them.

### Command line

```
gray secrets set NAME                                     store a secret (typed without echo)
gray secrets rm NAME                                      delete a stored secret
gray secrets list                                         names, lifetime, created, last used
gray secrets exec --env NAME[,NAME] [--file NAME] -- CMD  run CMD with secrets injected, output masked
gray secrets elevate-setup                                print the lines that enable gray-askpass
```

`--file NAME` writes the value to a 0600 file inside a 0700 directory under `$XDG_RUNTIME_DIR`, passes its path to the child as `NAME_FILE`, and deletes it on exit.

### Audit log

`~/.gray/secrets/audit.log` is append-only and records the time, session id, secret name, action (`set`, `use`, `deny`, `rm`, `request`) and the first 80 characters of the command with values masked. It never records values. Metadata for permanent secrets (created, last used) is kept in `~/.gray/secrets/index.json`.

### Elevation

`gray-askpass` is a sudo askpass helper. It reads the requesting command from its parent sudo process and the gray session id, shows both in a `/usr/bin/gnome-ssh-askpass` dialog, and appends an approved or cancelled line to the audit log. It never logs the password. `gray secrets elevate-setup` prints the lines to add to `~/.config/bspwm/bspwmrc` and `/etc/sudo.conf`; it does not change any file itself.

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
