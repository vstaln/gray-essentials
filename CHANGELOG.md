# Changelog

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
