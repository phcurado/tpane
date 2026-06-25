# tpane

tpane lets you improve your `tmux.conf` by moving most of your configuration to Lua. It ships with widgets and a plugin system: use widgets to improve your navbar, and plugins to improve your workflow.

## What it looks like

```lua
-- ~/.config/tmux/tpane/init.lua
tpane.use("vim-navigator") -- vim-style pane navigation
tpane.use("yank")      -- copy-mode clipboard helpers

-- options
tpane.opt.mouse = true
tpane.opt.history_limit = 5000
tpane.opt.mode_keys = "vi"

-- keybinds
tpane.bind("h", tpane.pane.select("left"))
tpane.bind("j", tpane.pane.select("down"))
tpane.bind("k", tpane.pane.select("up"))
tpane.bind("l", tpane.pane.select("right"))
tpane.bind("%", tpane.pane.split("right", { cwd = "pane" }))
tpane.bind('"', tpane.pane.split("down", { cwd = "pane" }))

-- navbar
tpane.widget("project", function(ctx)
  return ctx.pane and ctx.pane.cwd_basename or ""
end)

tpane.statusline {
  position = "top",
  left = { "session" },
  right = { "project", "clock" },
}
```

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/phcurado/tpane/main/install.sh | sh
```

From source:

```sh
cargo install --path . --locked --force
```

## Minimal tmux.conf

Only a few settings are necessary to live in `tmux.conf` file. These are the settings that are good to start with tmux, and `tpane` will have the runtime config:

```tmux
set -g default-terminal "xterm-256color"
set -as terminal-features ",xterm-256color:RGB"

set -g base-index 1
set -g pane-base-index 1

set -g status-position top
set -g status-style bg=default

unbind C-b
set -g prefix C-a
bind C-a send-prefix

# Add tpane here so you can configure all the rest in lua
run-shell -b 'tpane'
```

## Config location

tpane loads top-level Lua files from:

```text
~/.config/tmux/tpane
```

Set `TPANE_CONFIG_DIR` to use another directory.

## Replace tmux config with Lua

Use `tpane.opt` for tmux options:

```lua
tpane.opt.mouse = true
tpane.opt.history_limit = 5000
tpane.opt.mode_keys = "vi"
tpane.opt.renumber_windows = true
tpane.opt.escape_time = 0
```

tmux has options where you usually add one value without replacing the existing values. In tmux.conf that looks like:

```tmux
set -ga update-environment TERM
set -ga update-environment TERM_PROGRAM
```

In Lua, use `tpane.append` for the same thing:

```lua
tpane.append("update_environment", "TERM")
tpane.append("update_environment", "TERM_PROGRAM")
```

Bind keys with tmux-aware actions:

```lua
tpane.bind("h", tpane.pane.select("left"))
tpane.bind("j", tpane.pane.select("down"))
tpane.bind("k", tpane.pane.select("up"))
tpane.bind("l", tpane.pane.select("right"))

tpane.bind("%", tpane.pane.split("right", { cwd = "pane" }))
tpane.bind('"', tpane.pane.split("down", { cwd = "pane" }))

tpane.bind("M-Left", tpane.pane.resize("left", 10), { prefix = false })
```

If some configuration is not supported by `tpane`, you can always write it the same way you would in tmux:

```lua
tpane.bind("R", "source-file ~/.config/tmux/tmux.conf ; display 'reloaded'")
```

## Status bar and tabs

`tpane` lets you compose the statusline with widgets. It's very simple to add widgets to your statusline, and you can extend it or even create plugins for it (more on that in the next section).

```lua
tpane.widget("session", function(ctx)
  return "[" .. ctx.session .. "]"
end)

tpane.widget("clock", function()
  return os.date("%H:%M")
end)

tpane.widget("host", function()
  return os.getenv("HOSTNAME") or ""
end)

tpane.statusline {
  position = "top",
  left = { "session" },
  right = { "host", "clock" },
}
```

For widgets that run shell commands, use `job`. Jobs run in the background and return a handle that widgets can render:

```lua
local uptime = tpane.job("uptime", { every = "1m", timeout = "5s", cmd = "uptime" })

tpane.widget("uptime", function()
  return uptime
end)
```

Style tmux window tabs without writing the full tmux format by hand:

```lua
tpane.tabline {
  label = "cwd",
  inactive = { fg = "#777777" },
  current = { fg = "#8caaee", bold = true },
}
```

For lower-level styling, use nested tmux options:

```lua
tpane.options {
  status = { style = { bg = "default" } },
  pane = { border = { style = { fg = "#51576d" } } },
}
```

## Plugins

Plugins are referenced from Lua. Built-in plugins load by name:

```lua
tpane.use("vim-navigator")
tpane.use("yank")
```

Git plugins install when first referenced:

```lua
tpane.use("theme", {
  repo = "https://github.com/example/tpane-theme.git",
  branch = "main",
})
```

You can also reference a path in case the plugin is in a monorepo:

```lua
tpane.use("tool", {
  repo = "https://github.com/example/tools.git",
  path = "plugins/tpane-tool",
})
```

And use the CLI to keep track of your plugins:

```sh
tpane plugin status      # show referenced, installed, dirty, and update state
tpane plugin sync        # install/update plugins referenced by Lua config
tpane plugin update      # update all installed plugins
tpane plugin update NAME # update one plugin
tpane plugin clean       # remove installed plugins not referenced by Lua config
tpane plugin list        # list installed git plugins
tpane plugin remove NAME # remove one installed plugin
```

## Reusable panes

Register a pane once, then toggle or expand it from keybinds. Hidden panes keep
their process running.

```lua
tpane.register_pane("logs", {
  side = "bottom",
  size = "25%",
  command = "tail -f logs/app.log",
})

-- Show/Hide a pane 
tpane.bind("L", function(pane)
  tpane.toggle(pane, "logs")
end)
```

## CLI

```sh
tpane          # start or reload the daemon from inside tmux
tpane status   # show load/runtime errors
tpane reload   # reload Lua config
tpane refresh  # reload and rescan panes
tpane doctor   # inspect hidden panes/sessions
tpane update   # update tpane
tpane run NAME # run a Lua command
```

Full Lua reference: [`docs/lua-api.md`](docs/lua-api.md).
