# Plugins

Plugins are loaded from Lua with `tpane.use`.

```lua
tpane.use("sensible")
tpane.use("vim-navigator")
tpane.use("yank")
tpane.use("themes")
tpane.use("pane-detection")
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

### open-url

Open the URL under the cursor without selecting it first.

```lua
tpane.use("open-url")
```

Binds `<prefix> o` in normal mode and `C-o` in copy mode. Supports `https://`, `http://`, `www.`, common bare domains and `localhost` URLs.

### agents

Show agent pane notifications directly in window tabs.

```lua
tpane.use("agents")
tpane.tabline({ label = "cwd" })
```

The plugin finds known agent panes and marks their window tab. By default, detected agents show as idle. Live state comes from the agent, not from scraping scrollback.

States:

| State     | Tab meaning                        |
| --------- | ---------------------------------- |
| `idle`    | agent is waiting for a prompt      |
| `working` | agent is running                   |
| `blocked` | agent needs approval or input      |
| `done`    | agent finished while you were away |

You can test it manually from inside an agent pane:

```sh
tpane set-state "$TMUX_PANE" working
tpane set-state "$TMUX_PANE" blocked
tpane set-state "$TMUX_PANE" done
tpane set-state "$TMUX_PANE" idle
```

Built-in matching covers Claude, Codex, OpenCode, Gemini, and Pi. They show as idle unless the agent is configured to report state to tpane.

For now, automatic state reporting is manual setup. Built-in setup helpers for Claude, Codex, and OpenCode are planned.

Custom agents can be added from Lua:

```lua
tpane.agents.register({
  name = "my-agent",
  label = "agent",
  commands = { "my-agent" },
})
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

### pane-detection

Basic pane labels and pane border titles by command.

```lua
tpane.use("pane-detection")
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
