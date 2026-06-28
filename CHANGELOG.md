# Changelog

## 0.5.5 - 2026-06-28

### Bug Fixes

- render single-line statuslines with `status-format[0]` so `tpane.widgets.tabs` is not truncated by `status-left-length`
- reduce default spacing between statusline widgets
- remove extra trailing space from the session widget

### Documentation

- add documentation site
- add README logo, badges and docs links

## 0.5.3 - 2026-06-27

### Features

- add `tpane --version`

### Documentation

- document targeted mise upgrade command

## 0.5.2 - 2026-06-27

### Features

- add built-in `sensible` plugin
- add `cpu(opts)` and `memory(opts)` widgets
- add plugin documentation
- allow statusline rows as top-level shorthand

### Bug Fixes

- apply statusline changes immediately on reload
- clear stale statusline rows when switching between multiline and single-line configs

## 0.5.1 - 2026-06-27

### Features

- add multiline statusline rows

## 0.5.0 - 2026-06-26

### Features

- add built-in `themes` plugin generated from iTerm2 color schemes
- add `tpane themes` to list bundled themes

## 0.4.2 - 2026-06-25

### Bug Fixes

- Lua commands are now internal to avoid plugin command name collisions

## 0.4.1 - 2026-06-25

### Bug Fixes

- generate job names internally instead of exposing public job names

## 0.4.0 - 2026-06-25

### Features

- add background jobs: `tpane.job(name, { every, timeout, cmd })`
- add built-in widgets: `session`, `host`, `clock`, `date`, `prefix`, `battery(opts)`, and `player(opts)`
- add built-in plugins: `vim-navigator` and `yank`
- add Lua helpers for tmux options, appends, unbinds, raw commands, and key bindings

### Bug Fixes

- make jobs time out and clear their running state
- avoid overwriting externally managed pane tags with detected kind tags
- clear stale applied state after tmux server restarts
- clear pane tags when Lua detection no longer returns that tag
