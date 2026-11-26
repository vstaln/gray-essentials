# Changelog

## [0.6.0] - 2026-11-25

### Added
- **Session names**: after a generated title is accepted, the plugin also names
  the session itself: a 2-4 word slug of the title with stop-words dropped,
  trimmed to 48 characters, ending in four hex digits of FNV-1a 64 over the
  session id (`acp-usage-cache-7f3a`). The name goes out over the new
  `host/session_name {name}` host method, declared under the existing
  `session.write` capability.
- **`session_rename` tool**: the agent renames its own session for the work at
  hand, in 2-4 words; the plugin slugifies them and keeps the hash suffix. It
  needs the `title` part, and answers with a plain error when the host does not
  provide `host/session_name`.

## [0.5.0] - 2026-11-24

### Changed
- Renamed the package to `gray-essentials` (`essentials`) and the overview command to `/essentials`. Parts, part commands and on-disk state are unchanged; `~/.gray/qol/` and `/qol` keep working, and settings still read `extras.json`.

## [0.4.0] - 2026-11-23

### Added
- **Skills (`gray-skills` folded into `gray-extras`)**: skill discovery across `~/.gray/skills`, `~/.claude/skills`, `~/.config/opencode`, `~/.agents/skills`, and project skill dirs; `<available_skills>` prompt listing with context-budgeted description clipping; `skill`, `skills_search`, and `skills_install` tools; `/skills` slash command with dynamic per-command completion publishing.
- **Secrets (`gray-secrets` folded into `gray-extras`)**: `secret_request` tool with masked inline terminal prompt (`host.secret` capability), `tool/before` placeholder rewriting into the `exec --env` wrapper, refusal of `secret-tool` and state-touching commands, required `tool/after` output masking (raw, base64, URL-encoded), 0600 session socket and reaper, `/secrets` slash command, `gray secrets set|rm|list|exec|elevate-setup` CLI routing, and `gray-askpass`.

## [0.3.0] - 2026-11-23

### Changed
- Renamed package to `gray-extras` (`extras`) and folded vim mode, themes, permissions, and questions.

## [0.2.0] - 2026-11-15

### Added
- **`host.links`**: the issue-link rules now declare their own capability instead of riding on `host.say`.
- **`/export`** writes the transcript to the clipboard, a markdown file or a text file.
- **`/cd` and `/chdir`** switch the session's working directory.
- **Settings rows** in `/settings` for every quality-of-life feature, so each one can be turned off.
- Template commands declared in the manifest, refreshed with `host/commands_changed`.

### Changed
- **The features moved out of gray core into this plugin**, on the host API: the away recap, auto-continue, the break reminder, notifications, auto-title, suggestions, prompt templates, issue links, the interrupted hint and session cleanup. Core only keeps the generic host methods.
- **`turn_end` reads a settled transcript** and accumulates live usage per turn.
- **Session pruning asks `host/sessions` for the full list** instead of guessing from the current project.
- The package is named `gray-quality-of-life`; the catalog name stays `quality-of-life`, the `/qol` command and `~/.gray/qol` data directory are unchanged.
- Slash-command descriptions come from the manifest.

## [0.1.3] - 2026-10-28

### Fixed
- Reports its catalog name so `gray plugin list` shows one entry.
