# Changelog

## 0.4.1 - 2026-06-25

### Bug Fixes

- make job handles private by removing public job names from `tpane.job`
- reject duplicate named commands

## 0.4.0 - 2026-06-25

### Features

- add background jobs: `tpane.job(name, { every, timeout, cmd })`
- add built-in widgets: `tpane.widgets`: `session`, `host`, `clock`, `date`, `prefix`, `battery(opts)`, and `player(opts)`
- add built-in plugins: `vim-navigator` and `yank`
- add `tpane run <command>` for Lua-defined commands from key bindings
- add Lua helpers for tmux options, appends, unbinds, raw commands

### Bug Fixes

- make jobs time out and clear their running state
- clear stale applied state after tmux server restarts
- clear pane tags when Lua detection no longer returns that tag
