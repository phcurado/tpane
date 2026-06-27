# tpane

tpane lets you improve your `tmux.conf` by moving most of your configuration to Lua. It ships with widgets and a plugin system: use widgets to improve your navbar, and plugins to improve your workflow.

## Demo

https://github.com/user-attachments/assets/2e92141d-1e6e-407e-903e-29453cbf95ff

## Quick start

Install tpane:

```sh
curl -fsSL https://raw.githubusercontent.com/phcurado/tpane/main/install.sh | sh
```

Add this as the last line of `~/.config/tmux/tmux.conf`:

```tmux
run-shell -b 'tpane'
```

Create `~/.config/tmux/tpane/init.lua`:

```lua
tpane.use("sensible")
tpane.use("themes")
tpane.use("vim-navigator")
tpane.use("yank")

tpane.theme("Catppuccin Mocha")

tpane.opt.mouse = true
tpane.opt.mode_keys = "vi"

tpane.bind("h", tpane.pane.select("left"))
tpane.bind("j", tpane.pane.select("down"))
tpane.bind("k", tpane.pane.select("up"))
tpane.bind("l", tpane.pane.select("right"))

local battery = tpane.widgets.battery({ every = "30s" })

tpane.statusline {
  position = "top",
  left = { tpane.widgets.session, tpane.widgets.tabs },
  right = { battery, tpane.widgets.clock, tpane.widgets.date, tpane.widgets.prefix },
}
```

## Install

From crates.io:

```sh
cargo install tpane
```

Or install the latest GitHub release with the install script:

```sh
curl -fsSL https://raw.githubusercontent.com/phcurado/tpane/main/install.sh | sh
```

From source:

```sh
cargo install --path . --locked --force
```

With mise:

```sh
mise use -g github:phcurado/tpane@latest
```

This installs from the GitHub release assets and lets mise manage updates:

```sh
mise upgrade github:phcurado/tpane
```

## Minimal tmux.conf

Only a few settings are necessary to live in `tmux.conf` file. These are the settings that are good to start with tmux, and `tpane` will have the runtime config:

```tmux
set -g default-terminal "xterm-256color"
set -as terminal-features ",xterm-256color:RGB"

set -g base-index 1
set -g pane-base-index 1

unbind C-b
set -g prefix C-a
bind C-a send-prefix

# Keep this last.
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

`tpane` lets you compose the statusline with widgets. It ships with common widgets, and you can add your own when you need something custom.

```lua
tpane.statusline {
  position = "top",
  left = { tpane.widgets.session, tpane.widgets.tabs },
  right = { tpane.widgets.host, tpane.widgets.clock },
}
```

`tpane.widgets.tabs` renders the tmux window list.

Use rows for a multiline status bar:

```lua
tpane.statusline {
  position = "top",
  rows = {
    { left = { tpane.widgets.session }, right = { tpane.widgets.clock } },
    { left = { tpane.widgets.tabs }, right = { tpane.widgets.prefix } },
  },
}
```

Built-in widgets:

| Widget                        | Description                                                 |
| ----------------------------- | ----------------------------------------------------------- |
| `tpane.widgets.session`       | Current tmux session.                                       |
| `tpane.widgets.host`          | Hostname from tmux.                                         |
| `tpane.widgets.clock`         | Current time, like `14:30`.                                 |
| `tpane.widgets.date`          | Current date, like `Jun 25`.                                |
| `tpane.widgets.prefix`        | Shows when tmux prefix is active.                           |
| `tpane.widgets.tabs`          | tmux window tabs.                                           |
| `tpane.widgets.cpu(opts)`     | CPU usage. Works on Linux and macOS.                        |
| `tpane.widgets.memory(opts)`  | Used memory. Works on Linux and macOS.                      |
| `tpane.widgets.battery(opts)` | Battery status with icons. Works on Linux and macOS.        |
| `tpane.widgets.player(opts)`  | Current playing track. Uses `playerctl`, Music, or Spotify. |

```lua
local cpu = tpane.widgets.cpu({ every = "2s" })
local memory = tpane.widgets.memory({ every = "5s" })
local battery = tpane.widgets.battery({ every = "30s" })
local player = tpane.widgets.player({ every = "5s" })

tpane.statusline {
  right = { player, cpu, memory, battery, tpane.widgets.clock },
}
```

Custom widgets are just Lua functions:

```lua
local cwd = tpane.widget(function(ctx)
  return ctx.pane and ctx.pane.cwd_basename or ""
end)
```

For widgets that run shell commands, use `job`. Jobs run in the background and return a handle that widgets can render:

```lua
local uptime = tpane.job({ every = "1m", timeout = "5s", cmd = "uptime" })

tpane.statusline {
  right = { uptime },
}
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

See [docs/plugins.md](docs/plugins.md) for plugin details.

Plugins are referenced from Lua. Built-in plugins load by name:

```lua
tpane.use("sensible")
tpane.use("vim-navigator")
tpane.use("yank")
tpane.use("themes")
```

The themes plugin bundles the iTerm2 Color Schemes collection:

```sh
tpane themes
```

```lua
tpane.use("themes")
tpane.theme("Catppuccin Mocha")
```

Keep the terminal background behind the status bar:

```lua
tpane.theme("Gruvbox Dark", { transparent = true })
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
```

Full Lua reference: [`docs/lua-api.md`](docs/lua-api.md).
