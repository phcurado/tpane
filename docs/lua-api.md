# Lua API

tpane loads top-level `*.lua` files under:

```text
~/.config/tmux/tpane
```

Set `TPANE_CONFIG_DIR` to use another directory.

Files in subdirectories are not auto-loaded. Use Lua's `require`:

```lua
local colors = require("theme.colors") -- ~/.config/tmux/tpane/theme/colors.lua
```

## Replacing tmux.conf

### Options

Use `tpane.opt` for normal tmux options:

```lua
tpane.opt.mouse = true              -- set -g mouse on
tpane.opt.history_limit = 5000      -- set -g history-limit 5000
tpane.opt.mode_keys = "vi"          -- set -g mode-keys vi
tpane.opt.renumber_windows = true   -- set -g renumber-windows on
tpane.opt.escape_time = 0           -- set -g escape-time 0
```

### Appending to tmux options

Some tmux options are usually extended instead of replaced. In `tmux.conf` that
looks like this:

```tmux
set -ga update-environment TERM
set -ga update-environment TERM_PROGRAM
```

In Lua, use `tpane.append`:

```lua
tpane.append("update_environment", "TERM")
tpane.append("update_environment", "TERM_PROGRAM")
```

### Key bindings

Use `tpane.bind(key, action[, opts])`:

```lua
tpane.bind("h", tpane.pane.select("left"))
tpane.bind("j", tpane.pane.select("down"))
tpane.bind("k", tpane.pane.select("up"))
tpane.bind("l", tpane.pane.select("right"))
```

By default, bindings uses tmux's prefix. Use `prefix = false` for keybindings that are not using tmux prefix:

```lua
tpane.bind("M-Left", tpane.pane.resize("left", 10), { prefix = false })
```

Use `mode = "copy"` for copy mode:

```lua
tpane.bind("v", tpane.copy.begin(), { mode = "copy" })
tpane.bind("r", tpane.copy.rectangle(), { mode = "copy" })
tpane.bind("y", tpane.copy.copy(), { mode = "copy" })
```

### Lua key handlers

A binding can run Lua. The handler receives the pane that invoked the binding:

```lua
tpane.bind("L", function(pane)
  tpane.toggle(pane, "logs")
end)
```

### Raw tmux commands

If tpane does not have a helper for something, you can write the tmux command directly:

```lua
tpane.bind("R", "source-file ~/.config/tmux/tmux.conf ; display 'reloaded'")
```

For multiple raw commands, use `tpane.raw` with a list:

```lua
tpane.bind("C-S-l", tpane.raw({
  "swap-window -t +1",
  "select-window -t +1",
}), { prefix = false })
```

### Unbinding

```lua
tpane.unbind("C-b")
tpane.unbind("v", { mode = "copy" })
```

## Actions

Actions are values you pass to `tpane.bind`.

### `tpane.run`

Run a Lua command registered with `tpane.command`:

```lua
tpane.bind("x", tpane.run("hello"))
tpane.bind("x", tpane.run({ "hello", "arg1" }))
```

### `tpane.raw`

Run raw tmux commands:

```lua
tpane.raw("select-pane -L")
tpane.raw({ "swap-window -t +1", "select-window -t +1" })
```

### Pane actions

```lua
tpane.pane.select("left")
tpane.pane.select("right")
tpane.pane.select("up")
tpane.pane.select("down")

tpane.pane.resize("left", 10)
tpane.pane.resize("right", 10)
tpane.pane.resize("up", 5)
tpane.pane.resize("down", 5)

tpane.pane.split("right", { cwd = "pane" })
tpane.pane.split("down", { cwd = "pane" })
tpane.pane.split("left")
tpane.pane.split("up")
```

`cwd = "pane"` means use the current pane's directory.

### Window actions

```lua
tpane.window.new({ cwd = "pane" })
tpane.window.swap("next")
tpane.window.swap("prev")
```

### Copy-mode actions

```lua
tpane.copy.begin()
tpane.copy.begin({ rectangle = true })
tpane.copy.rectangle()
tpane.copy.copy()
```

### Key actions

```lua
tpane.key.prefix()
```

## Status bar and tabs

### Widgets

A widget is a Lua function that returns text, a styled table, a list of parts, or
`nil` to hide itself.

```lua
tpane.widget("host", function()
  return os.getenv("HOSTNAME") or ""
end)

tpane.widget("mode", function(ctx)
  if ctx.pane and ctx.pane.zoomed then
    return { text = "zoom", fg = "yellow", bold = true }
  end
end)
```

Widget context:

```lua
ctx.session  -- current session name
ctx.window   -- current window id, like @2
ctx.pane     -- current pane object, or nil
ctx.panes    -- all pane objects
```

Built-in widgets:

```text
session
clock
companions
```

Raw tmux format strings also work:

```lua
tpane.widget("prefix", function()
  return tpane.fmt.prefix("PREFIX", "")
end)
```

### Statusline

```lua
tpane.statusline {
  position = "top",
  interval = 1,
  left = { "session" },
  right = { "host", "clock" },
  separator = "  ",
}
```

### Styled parts

```lua
return { text = "ok", fg = "green", bold = true }
```

Supported style keys:

```text
fg, bg, bold, dim, italics, blink, reverse, hidden, strikethrough, underscore,
align, fill
```

### Tabline

`tpane.tabline` writes the common `window-status-format` options for you:

```lua
tpane.tabline {
  label = "cwd", -- cwd, name, or a raw tmux format string
  inactive = { fg = "#777777" },
  current = { fg = "#8caaee", bold = true },
}
```

## Plugins

Plugins are referenced from Lua with `tpane.use`.

Built-in plugins:

```lua
tpane.use("vim-navigator")
tpane.use("yank")
tpane.use("agents")
```

Git plugins:

```lua
tpane.use("theme", {
  repo = "https://github.com/example/tpane-theme.git",
  branch = "main",
})
```

Monorepo plugin path:

```lua
tpane.use("tool", {
  repo = "https://github.com/example/tools.git",
  path = "plugins/tpane-tool",
})
```

`repo` is the git URL. `url` also works. `branch`, `tag`, and `rev` are mutually
exclusive. `path` is relative to the repo and uses sparse checkout.

Plugin commands:

```sh
tpane plugin status      # show referenced, installed, dirty, and update state
tpane plugin sync        # install/update plugins referenced by Lua config
tpane plugin update      # update all installed plugins
tpane plugin update NAME # update one plugin
tpane plugin clean       # remove installed plugins not referenced by Lua config
tpane plugin list        # list installed git plugins
tpane plugin remove NAME # remove one installed plugin
```

## Commands

Register commands when you want a Lua function callable from the CLI or from a
key binding.

```lua
tpane.command("hello", function(args)
  return "hello " .. (args[1] or "")
end)
```

Run it:

```sh
tpane run hello world
```

Bind it:

```lua
tpane.bind("H", tpane.run("hello"))
```

## Reusable panes

Register panes you want to show/hide later:

```lua
tpane.register_pane("logs", {
  side = "bottom",
  size = "25%",
  command = "tail -f logs/app.log",
})
```

Use key handlers to control it:

```lua
tpane.bind("L", function(pane)
  tpane.toggle(pane, "logs")
end)

tpane.bind("M-L", function(pane)
  tpane.expand(pane, "logs")
end, { prefix = false })
```

`toggle` shows or hides it. Hidden panes are stashed, so the process keeps
running. `expand` shows it and zooms the layout around it.

Options:

```lua
tag = "logs"                 -- defaults to registered name
name = "logs"                -- stash name, defaults to registered name
side = "bottom"              -- bottom | top | right | left
size = "25%"
full = true                  -- split across the full window
anchor = { tag = "editor" }  -- optional target pane for split/unstash
command = "tail -f app.log"
title = "logs"
label = "logs"
blocked_message = "..."      -- shown instead of hiding a blocked pane
```

Use `tpane.split` when you want a one-off split instead of a registered pane:

```lua
local pane = tpane.split(current, {
  side = "bottom",
  size = "25%",
  command = "zsh",
})
```

## Pane objects

Kind callbacks, key handlers, events, widgets, and `tpane.panes()` use pane
objects.

Fields:

```lua
pane.id            -- tmux pane id, like %3
pane.pid           -- root process pid
pane.cwd           -- current directory
pane.cwd_basename  -- last path component of cwd
pane.command       -- tmux pane_current_command
pane.session       -- session name
pane.window        -- window id, like @2
pane.active        -- true if focused
pane.zoomed        -- true if window is zoomed
pane.kind          -- detected kind
pane.label         -- shown label
pane.tag           -- user tag set by tpane
pane.home          -- home window for stashed panes
pane.state         -- current state, if any
```

Methods:

```lua
pane:running("psql")
pane:var("@tmux_var")
pane:set { tag = "logs", label = "logs" }
pane:capture()
pane:proc_tree()
```

Process tree example:

```lua
pane:proc_tree():any(function(proc)
  return proc.argv:match("--watch") ~= nil
end)
```

Find panes:

```lua
local logs = tpane.find { tag = "logs" }
local all_logs = tpane.find_all { tag = "logs" }
```

All fields in the query must match.

## Kinds and states

A kind tells tpane what a pane is.

```lua
tpane.kind { name = "database", match = "psql" }
```

When a pane is running `psql`:

```lua
pane.kind  -- database
pane.label -- database
```

Use `detect` for custom matching:

```lua
tpane.kind {
  name = "server",
  detect = function(pane)
    return pane:running("node") and pane.cwd:match("/server$") ~= nil
  end,
}
```

Use `label` to change what is shown:

```lua
tpane.kind {
  name = "editor",
  match = "nvim",
  label = function(pane)
    return "editor " .. pane.cwd_basename
  end,
}
```

Kinds can report state:

```lua
tpane.kind {
  name = "worker",
  match = "worker",
  state = function(pane)
    if pane:capture():match("blocked") then return "blocked" end
    if pane:capture():match("running") then return "working" end
    return "idle"
  end,
}
```

Built-in state presentations:

```text
approval
blocked
working
done_unseen
idle_seen
```

Declare custom state presentation:

```lua
tpane.state("waiting", { color = "yellow", glyph = "…" })
local presentation = tpane.state("waiting")
```

Returning `done` from detection is treated as `done_unseen` until the pane is
focused.

## Store

`tpane.store` is a small JSON-backed store for config and plugins.

```lua
tpane.store.set("counter", 1)
local value = tpane.store.get("counter")
tpane.store.delete("counter")
```

Values may be strings, numbers, booleans, tables, or nil.

## Events

```lua
tpane.on("tick", function() end)
tpane.on("pane:new", function(pane) end)
tpane.on("pane:focus", function(pane) end)
tpane.on("window:close", function(window_id) end)
tpane.on("state:change", function(pane_id) end)
```

## Panels

Panels are simple TUI views shown by `tpane control`.

```lua
tpane.panel {
  id = "tools",
  title = "Tools",
  cards = function()
    return {
      { title = "hello", tag = "command", enter = tpane.run("hello") },
    }
  end,
}
```

## Workspaces

Declare reusable layouts in Lua:

```lua
tpane.workspace {
  name = "dev",
  windows = {
    { name = "app", command = "zsh" },
    { name = "logs", panes = { { side = "bottom", size = "30%", command = "tail -f app.log" } } },
  },
}

tpane.command("dev", function()
  tpane.apply_workspace("dev")
end)
```

## Format helpers

Use `tpane.fmt` for tmux conditionals that do not have Lua equivalents:

```lua
tpane.fmt.prefix("", "")
tpane.fmt.when("window_zoomed_flag", "Z", "")
```

## Low-level tmux helpers

Use these when the higher-level helpers are not enough:

```lua
local window = tpane.tmux.new_window { name = "logs", cwd = pane.cwd, command = "zsh" }
tpane.tmux.select_window(window)
tpane.tmux.send_keys { target = pane.id, keys = "npm test", enter = true }
tpane.tmux.split { target = pane.id, dir = "below", size = "25%", cwd = pane.cwd }
tpane.tmux.stash { pane = pane.id, window = pane.window, cwd = pane.cwd, name = "hidden" }
tpane.tmux.unstash { pane = hidden.id, target = pane.id, horizontal = true, size = "35%" }
tpane.tmux.unzoom(pane.window)
tpane.tmux.select(pane.id)
tpane.tmux.zoom(pane.id)
tpane.tmux.display { target = pane.id, message = "message" }
```

## Public API

Main API:

```text
tpane.use
tpane.opt
tpane.append
tpane.options
tpane.bind
tpane.unbind
tpane.run
tpane.raw
tpane.pane.*
tpane.window.*
tpane.copy.*
tpane.key.*
tpane.widget
tpane.statusline
tpane.tabline
tpane.command
tpane.panel
tpane.register_pane
tpane.split
tpane.toggle
tpane.show
tpane.hide
tpane.expand
tpane.panes
tpane.find
tpane.find_all
tpane.kind
tpane.state
tpane.on
tpane.store
tpane.tmux
tpane.fmt
```
