# Plugins

Plugins are loaded from Lua with `tpane.use`.

```lua
tpane.use("sensible")
tpane.use("vim-navigator")
tpane.use("yank")
tpane.use("themes")
```

## Built-in plugins

### sensible

Common tmux defaults:

```lua
tpane.use("sensible")
```

Sets:

```lua
tpane.options {
  escape_time = 0,
  history_limit = 50000,
  display_time = 4000,
  status_interval = 5,
  focus_events = true,
  status_keys = "emacs",
  aggressive_resize = true,
}
```

### vim-navigator

Use Vim-style pane navigation. If the active process looks like Vim, the key is sent to the pane. Otherwise tmux moves panes.

```lua
tpane.use("vim-navigator")
```

Binds `C-h`, `C-j`, `C-k`, and `C-l`.

### yank

Copy-mode bindings for yanking selected text and pane paths.

```lua
tpane.use("yank")
```

### themes

Bundled themes from the iTerm2 Color Schemes collection.

```lua
tpane.use("themes")
tpane.theme("Gruvbox Dark", { transparent = true })
```

List themes:

```sh
tpane themes
```

## Git plugins

Use a git plugin by passing a spec:

```lua
tpane.use("tool", {
  repo = "https://github.com/example/tpane-tool.git",
  branch = "main",
})
```

If the plugin lives in a monorepo:

```lua
tpane.use("tool", {
  repo = "https://github.com/example/tools.git",
  path = "plugins/tpane-tool",
})
```

Manage installed git plugins:

```sh
tpane plugin status
tpane plugin sync
tpane plugin update
tpane plugin clean
tpane plugin list
tpane plugin remove NAME
```
