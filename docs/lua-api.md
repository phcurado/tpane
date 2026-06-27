# Lua API

This page documents what you can use inside `~/.config/tmux/tpane/*.lua`.
Use it as a reference when you want to move tmux options, key bindings, status
bar config, plugins, pane helpers, or small bits of behavior into Lua.

A minimal setup looks like this:

```tmux
# ~/.config/tmux/tmux.conf
run-shell -b 'tpane'
```

```lua
-- ~/.config/tmux/tpane/init.lua
tpane.opt.mouse = true
tpane.bind("h", tpane.pane.select("left"))
tpane.statusline { right = { tpane.widgets.clock } }
```

Reload from inside tmux with:

```sh
tpane reload
```

Check errors with:

```sh
tpane status
```

## Config files

tpane loads top-level Lua files from:

```text
~/.config/tmux/tpane
```

Set `TPANE_CONFIG_DIR` to load config from somewhere else.

Files in subdirectories are not loaded automatically. Use `require` for shared
modules:

```lua
local colors = require("theme.colors") -- ~/.config/tmux/tpane/theme/colors.lua
```

## Options

Use `tpane.opt` for normal tmux options:

```lua
tpane.opt.mouse = true              -- set -g mouse on
tpane.opt.history_limit = 5000      -- set -g history-limit 5000
tpane.opt.mode_keys = "vi"          -- set -g mode-keys vi
tpane.opt.renumber_windows = true   -- set -g renumber-windows on
tpane.opt.escape_time = 0           -- set -g escape-time 0
```

Use `tpane.append` when you would use `set -ga` in tmux:

```lua
tpane.append("update_environment", "TERM")
tpane.append("update_environment", "TERM_PROGRAM")
```

`tpane.options` is the table form of the same API. Use it when you want to set
several related options together:

```lua
tpane.options {
  status_style = { bg = "default" },
  pane_border_style = { fg = "#51576d" },
}
```

Nested tables are flattened into tmux option names, so this is equivalent:

```lua
tpane.options {
  status = { style = { bg = "default" } },
  pane = { border = { style = { fg = "#51576d" } } },
}
```

## Key bindings

`tpane.bind` connects a key to an action.

```lua
tpane.bind("h", tpane.pane.select("left"))
tpane.bind("j", tpane.pane.select("down"))
tpane.bind("k", tpane.pane.select("up"))
tpane.bind("l", tpane.pane.select("right"))
```

Bindings use the tmux prefix by default. Pass `prefix = false` for root bindings:

```lua
tpane.bind("M-Left", tpane.pane.resize("left", 10), { prefix = false })
```

Copy-mode bindings go in tmux's copy-mode key table. This example enters copy
mode with `<prefix> [` and then uses `v`, `r`, and `y` inside copy mode:

```lua
tpane.bind("[", "copy-mode")
tpane.bind("v", tpane.copy.begin(), { mode = "copy" })
tpane.bind("r", tpane.copy.rectangle(), { mode = "copy" })
tpane.bind("y", tpane.copy.copy(), { mode = "copy" })
```

`mode = "copy"` maps to tmux's `copy-mode-vi` table. If you want another tmux
key table, pass it directly:

```lua
tpane.bind("y", tpane.copy.copy(), { table = "copy-mode" })
```

A binding can also run a Lua functions. The callback receives the pane that pressed the key:

```lua
tpane.bind("L", function(pane)
  tpane.toggle(pane, "logs")
end)
```

Use raw tmux commands when there is no helper:

```lua
tpane.bind("R", "source-file ~/.config/tmux/tmux.conf ; display 'reloaded'")
```

For multiple raw commands, use `tpane.raw`:

```lua
tpane.bind("C-S-l", tpane.raw({
  "swap-window -t +1",
  "select-window -t +1",
}), { prefix = false })
```

Remove a binding with `tpane.unbind`:

```lua
tpane.unbind("C-b")
tpane.unbind("v", { mode = "copy" })
```

## Actions

Actions are values you pass to `tpane.bind`. They are helpers for common tmux
commands.

Pane actions:

```lua
-- Move between panes.
tpane.bind("h", tpane.pane.select("left"))
tpane.bind("j", tpane.pane.select("down"))
tpane.bind("k", tpane.pane.select("up"))
tpane.bind("l", tpane.pane.select("right"))

-- Resize panes without the prefix.
tpane.bind("M-Left", tpane.pane.resize("left", 10), { prefix = false })
tpane.bind("M-Right", tpane.pane.resize("right", 10), { prefix = false })

-- Split panes.
tpane.bind("%", tpane.pane.split("right", { cwd = "pane" }))
tpane.bind('"', tpane.pane.split("down", { cwd = "pane" }))
```

`cwd = "pane"` is tpane shorthand for tmux's `-c "#{pane_current_path}"`.
Use it with `tpane.pane.split` or `tpane.window.new` when the new pane/window
should start in the current pane's directory.

Window actions:

```lua
-- New window in the current pane's directory.
tpane.bind("c", tpane.window.new({ cwd = "pane" }))

-- Reorder the current window.
tpane.bind(">", tpane.window.swap("next"))
tpane.bind("<", tpane.window.swap("prev"))
```

Copy-mode actions:

```lua
-- Enter copy mode with <prefix> [, then select/copy with vi-like keys.
tpane.bind("[", "copy-mode")
tpane.bind("v", tpane.copy.begin(), { mode = "copy" })
tpane.bind("r", tpane.copy.rectangle(), { mode = "copy" })
tpane.bind("y", tpane.copy.copy(), { mode = "copy" })
```

`tpane.copy.begin({ rectangle = true })` starts selection and toggles rectangle
mode in one binding:

```lua
tpane.bind("C-v", tpane.copy.begin({ rectangle = true }), { mode = "copy" })
```

Send the prefix key to the pane:

```lua
-- Optional. This makes <prefix> C-a send C-a to the program in the pane.
-- Useful for nested tmux sessions.
tpane.bind("C-a", tpane.key.prefix())
```

This is the Lua version of tmux's `bind C-a send-prefix`. You only need it if
you want a way to pass the prefix key through to the pane.

## Status bar

`tpane.statusline` defines what appears on the left and right side of the tmux
status bar.

```lua
tpane.statusline {
  position = "top",
  left = { tpane.widgets.session, tpane.widgets.tabs },
  right = { tpane.widgets.clock },
}
```

The values in `left` and `right` are widgets. A widget can be a built-in widget,
a custom Lua widget, or a job-backed widget.

### Built-in widgets

| Widget                        | Description                                                |
| ----------------------------- | ---------------------------------------------------------- |
| `tpane.widgets.session`       | Current tmux session.                                      |
| `tpane.widgets.host`          | Hostname from tmux.                                        |
| `tpane.widgets.clock`         | Current time, like `14:30`.                                |
| `tpane.widgets.date`          | Current date, like `Jun 25`.                               |
| `tpane.widgets.prefix`        | Shows when the tmux prefix key is active.                  |
| `tpane.widgets.tabs`          | tmux window list.                                          |
| `tpane.widgets.cpu(opts)`     | CPU usage. Works on Linux and macOS.                       |
| `tpane.widgets.memory(opts)`  | Used memory. Works on Linux and macOS.                     |
| `tpane.widgets.battery(opts)` | Battery status with icons. Works on Linux and macOS.       |
| `tpane.widgets.player(opts)`  | Current playing track. Uses `playerctl`, Music or Spotify. |

Plain widgets are used directly:

```lua
tpane.statusline {
  left = { tpane.widgets.session, tpane.widgets.tabs },
  right = { tpane.widgets.clock },
}
```

Widgets that take `opts` start background work. Call them once, store the result,
and use that result in the statusline:

```lua
local cpu = tpane.widgets.cpu({ every = "2s" })
local memory = tpane.widgets.memory({ every = "5s" })
local battery = tpane.widgets.battery({ every = "30s" })

tpane.statusline {
  right = { cpu, memory, battery, tpane.widgets.clock },
}
```

### Custom widgets

Use `tpane.widget` when you want to render your own widget. The function runs when
tpane renders the status bar.

```lua
local cwd = tpane.widget(function(ctx)
  return ctx.pane and ctx.pane.cwd_basename or ""
end)

tpane.statusline {
  left = { tpane.widgets.session, cwd },
  right = { tpane.widgets.clock },
}
```

The `ctx` argument contains the tmux state available while rendering:

| Field         | Description                      |
| ------------- | -------------------------------- |
| `ctx.session` | Current session name.            |
| `ctx.window`  | Current window id, like `@2`.    |
| `ctx.pane`    | Current pane object, or `nil`.   |
| `ctx.panes`   | All pane objects known to tpane. |

A widget can return a string:

```lua
local server = tpane.widget(function()
  return "server"
end)
```

To style the text, return a table with `text` plus style fields:

```lua
local status = tpane.widget(function()
  return { text = "ok", fg = "green", bold = true }
end)
```

That renders `ok` in green and bold in the status bar. Return `nil` or an empty
string to show nothing.

Style fields:

| Field           | Value             | Description                                               |
| --------------- | ----------------- | --------------------------------------------------------- |
| `text`          | string            | Text to show.                                             |
| `fg`            | string            | Foreground color, like `"green"` or `"#8caaee"`.          |
| `bg`            | string            | Background color.                                         |
| `bold`          | boolean           | Bold text.                                                |
| `dim`           | boolean           | Dim text.                                                 |
| `italics`       | boolean           | Italic text.                                              |
| `blink`         | boolean           | Blinking text.                                            |
| `reverse`       | boolean           | Swap foreground and background.                           |
| `hidden`        | boolean           | Hidden text.                                              |
| `strikethrough` | boolean           | Struck-through text.                                      |
| `underscore`    | boolean or string | Underline. Strings are passed to tmux as underline style. |
| `align`         | string            | tmux alignment style value.                               |
| `fill`          | string            | tmux fill style value.                                    |

### Shell command widgets

Use `tpane.job` for status bar data that comes from a shell command. Jobs run in
the background on their own interval, so the status bar does not block while the
command is running.

```lua
local uptime = tpane.job({
  every = "1m",
  timeout = "5s",
  cmd = "uptime",
})

tpane.statusline {
  right = { uptime },
}
```

`every` and `timeout` can be seconds or a string ending in `s`, `m`, or `h`.
`timeout` defaults to `10s`.

### Multiline status bar

Use `rows` for a multiline status bar:

```lua
tpane.statusline {
  position = "top",
  rows = {
    { left = { tpane.widgets.session }, right = { tpane.widgets.clock } },
    { left = { tpane.widgets.tabs }, right = { tpane.widgets.prefix } },
  },
}
```

## Window tabs

Use `tpane.tabline` to configure tmux window tabs:

```lua
tpane.tabline {
  label = "cwd", -- cwd, name, or a raw tmux format string
  inactive = { fg = "#777777" },
  current = { fg = "#8caaee", bold = true },
}
```

## Plugins and themes

Use `tpane.use` to load a plugin from your Lua config:

```lua
tpane.use("sensible")
tpane.use("vim-navigator")
tpane.use("yank")
tpane.use("themes")
```

External plugins use the same function with a repo spec:

```lua
tpane.use("theme", {
  repo = "https://github.com/example/tpane-theme.git",
  branch = "main",
})
```

See [plugins.md](plugins.md) for the built-in plugins, git plugin options, and
plugin management commands.

Themes are applied with `tpane.theme` after loading the `themes` plugin:

```lua
tpane.use("themes")
tpane.theme("Catppuccin Mocha")
tpane.theme("Gruvbox Dark", { transparent = true })
```

List bundled themes with:

```sh
tpane themes
```

## Reusable panes

Reusable panes are named panes that tpane can show, hide, and restore. Use them
for logs, REPLs, shells, database consoles, or watchers that should keep running
when hidden.

Register the pane once:

```lua
tpane.register_pane("logs", {
  side = "bottom",
  size = "25%",
  command = "tail -f logs/app.log",
})
```

Control it from a key binding:

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

Use `tpane.split` for a one-off split instead of a registered pane:

```lua
local pane = tpane.split(current, {
  side = "bottom",
  size = "25%",
  command = "zsh",
})
```

## Pane objects

A pane object is how tpane represents one tmux pane in Lua. It gives you the
pane id, current directory, command, window, state, and helper methods.

tpane passes pane objects to callbacks that work with a specific pane:

```lua
-- Key handler: pane is the pane that pressed the key.
tpane.bind("L", function(pane)
  tpane.toggle(pane, "logs")
end)

-- Widget: ctx.pane is the pane being used to render the status bar.
local cwd = tpane.widget(function(ctx)
  return ctx.pane and ctx.pane.cwd_basename or ""
end)

-- Kind detection: pane is each pane tpane is scanning.
tpane.kind {
  name = "editor",
  detect = function(pane)
    return pane:running("nvim")
  end,
}
```

### Pane fields

| Field               | Description                                                            |
| ------------------- | ---------------------------------------------------------------------- |
| `pane.id`           | tmux pane id, like `%3`. Use this when calling low-level tmux helpers. |
| `pane.pid`          | Root process pid for the pane.                                         |
| `pane.cwd`          | Current directory of the pane.                                         |
| `pane.cwd_basename` | Last path component of `pane.cwd`.                                     |
| `pane.command`      | tmux `pane_current_command`, usually the foreground command name.      |
| `pane.session`      | tmux session name.                                                     |
| `pane.window`       | tmux window id, like `@2`.                                             |
| `pane.active`       | `true` when the pane is focused.                                       |
| `pane.zoomed`       | `true` when the pane's window is zoomed.                               |
| `pane.kind`         | Kind detected by `tpane.kind`, if any.                                 |
| `pane.label`        | Label shown by tpane for this pane, if any.                            |
| `pane.tag`          | Custom tag stored on the pane. Useful for finding panes later.         |
| `pane.home`         | Home window for a stashed pane.                                        |
| `pane.state`        | Current pane state, if one was detected or set.                        |

### Pane methods

| Method               | Description                                                                  |
| -------------------- | ---------------------------------------------------------------------------- |
| `pane:running(name)` | Returns `true` if the pane's process tree contains a command with that name. |
| `pane:var(name)`     | Reads a tmux pane variable, such as `@my_var` or `@tpane_tag`.               |
| `pane:set(values)`   | Sets tpane pane metadata: `kind`, `label`, `state`, `tag`, or `home`.        |
| `pane:capture()`     | Returns the pane's visible text, like `tmux capture-pane`.                   |
| `pane:proc_tree()`   | Returns the pane's process tree. Use it for custom process checks.           |

Examples:

```lua
-- Label panes running psql.
tpane.kind {
  name = "database",
  detect = function(pane)
    return pane:running("psql")
  end,
}

-- Show the current directory in the status bar.
local cwd = tpane.widget(function(ctx)
  return ctx.pane and ctx.pane.cwd_basename or ""
end)

-- Mark the current pane so it can be found later.
tpane.bind("T", function(pane)
  pane:set { tag = "terminal", label = "terminal" }
end)

-- Jump back to the marked pane.
tpane.bind("G", function()
  local terminal = tpane.find { tag = "terminal" }
  if terminal then
    tpane.tmux.select(terminal.id)
  end
end)
```

Use `pane:capture()` to read the visible text in a pane. This is useful when the
process name is not enough and the output tells you what is happening:

```lua
-- Mark a worker pane as blocked/working by reading its output.
tpane.kind {
  name = "worker",
  match = "worker",
  state = function(pane)
    local text = pane:capture()
    if text:match("blocked") then return "blocked" end
    if text:match("running") then return "working" end
  end,
}
```

Use `pane:proc_tree()` to inspect all processes running under the pane. This is useful for commands hidden behind shells,
scripts, package managers, etc:

```lua
-- Detect a test watcher even if the pane command is just `zsh` or `npm`.
tpane.kind {
  name = "test watcher",
  detect = function(pane)
    return pane:proc_tree():any(function(proc)
      return proc.argv:match("--watch") ~= nil
    end)
  end,
}
```

A process has:

| Field       | Description        |
| ----------- | ------------------ |
| `proc.pid`  | Process id.        |
| `proc.ppid` | Parent process id. |
| `proc.argv` | Full command line. |

Use `tpane.find` when you want to do something with a pane you tagged or
identified earlier:

```lua
tpane.bind("G", function()
  local logs = tpane.find { tag = "logs" }
  if logs then
    tpane.tmux.select(logs.id)
  end
end)
```

`tpane.find` returns the first match. `tpane.find_all` returns every match. All
fields in the query must match.

## Kinds and states

Use kinds when you want tpane to recognize what a pane is and reuse that
information elsewhere.

For example let's say you want to label a panel that if it's running `psql`, it should be labeled as `database`:

```lua
tpane.kind { name = "database", match = "psql" }
```

Now you can act on that pane without remembering its tmux id:

```lua
tpane.bind("D", function()
  local db = tpane.find { kind = "database" }
  if db then
    tpane.tmux.select(db.id)
  end
end)
```

You can also show the recognized pane type in the status bar:

```lua
local pane_label = tpane.widget(function(ctx)
  return ctx.pane and ctx.pane.label or ""
end)
```

### Matching panes

Use `match` for a simple command-name check:

```lua
-- Any pane running nvim becomes an editor pane.
tpane.kind { name = "editor", match = "nvim" }
```

Use `detect` when the check needs more than a command name. It receives a pane
and returns `true` when the pane should get that kind:

```lua
-- A node process only counts as a server when it is in a server directory.
tpane.kind {
  name = "server",
  detect = function(pane)
    return pane:running("node") and pane.cwd:match("/server$") ~= nil
  end,
}
```

### Labels

The label is the display text for a matched pane. By default it is the kind name.
Use `label` when the display text should include pane-specific information:

```lua
tpane.kind {
  name = "editor",
  match = "nvim",
  label = function(pane)
    return "editor " .. pane.cwd_basename
  end,
}
```

If Neovim is running in `/home/me/project`, the label is `editor project`.

### States

A state is a short status attached to a pane. Use it when the pane can be
working, blocked, waiting, done, etc.

```lua
-- Read a worker pane's output and expose it as pane.state.
tpane.kind {
  name = "worker",
  match = "worker",
  state = function(pane)
    local text = pane:capture()
    if text:match("blocked") then return "blocked" end
    if text:match("running") then return "working" end
    return "idle"
  end,
}
```

Then use that state in a widget:

```lua
local state = tpane.widget(function(ctx)
  return ctx.pane and ctx.pane.state or ""
end)
```

Built-in states:

| State         | Meaning                                          |
| ------------- | ------------------------------------------------ |
| `approval`    | Waiting for approval.                            |
| `blocked`     | Blocked or needs attention.                      |
| `working`     | Work is in progress.                             |
| `done_unseen` | Finished, but the pane has not been focused yet. |
| `idle_seen`   | Finished/idle and already seen.                  |

If a state function returns `done`, tpane shows it as `done_unseen` until the
pane is focused.

Add your own state style with `tpane.state`:

```lua
tpane.state("waiting", { color = "yellow", glyph = "…" })
```

## Store

`tpane.store` persists small Lua values across reloads.

It is shared by the whole tpane config for the current tmux server. It is not
scoped per plugin, so choose keys that will not collide with other config or
plugins.

```lua
tpane.store.set("my-plugin.counter", 1)
tpane.store.set("layout.last-project", "tpane")
```

Avoid generic keys in plugins:

```lua
tpane.store.set("counter", 1)
tpane.store.set("enabled", true)
```

Read and delete values with the same key:

```lua
local count = tpane.store.get("my-plugin.counter")
tpane.store.delete("my-plugin.counter")
```

Values may be strings, numbers, booleans, tables, or nil.

## Events

Use `tpane.on` to register callbacks for tmux changes. The callback is called
when that event happens.

| Event          | Callback argument    | When it fires                 |
| -------------- | -------------------- | ----------------------------- |
| `tick`         | none                 | On each tpane scan.           |
| `pane:new`     | pane object          | When tpane sees a new pane.   |
| `pane:focus`   | pane object          | When the active pane changes. |
| `window:close` | window id, like `@2` | When a window disappears.     |
| `state:change` | pane id, like `%3`   | When a pane state changes.    |

Example: show a tmux message when focusing a pane recognized as a database:

```lua
tpane.on("pane:focus", function(pane)
  if pane.kind == "database" then
    tpane.tmux.display { target = pane.id, message = "database pane" }
  end
end)
```

Example: keep a counter of how many panes were created in this tmux server:

```lua
tpane.on("pane:new", function()
  local count = tpane.store.get("stats.panes-created") or 0
  tpane.store.set("stats.panes-created", count + 1)
end)
```

Example: react when a worker pane changes state:

```lua
tpane.on("state:change", function(pane_id)
  local pane = tpane.pane(pane_id)
  if pane.state == "blocked" then
    tpane.tmux.display { target = pane.id, message = "worker is blocked" }
  end
end)
```

Keep event callbacks short. If you need to run a shell command on a timer, use
`tpane.job` instead of `tick`.

## Workspaces

A workspace is a named tmux layout. Register it in Lua, then apply it from a key
binding or command.

```lua
tpane.workspace {
  name = "dev",
  windows = {
    { name = "app", command = "zsh" },
    {
      name = "logs",
      panes = {
        { side = "bottom", size = "30%", command = "tail -f app.log" },
      },
    },
  },
}

tpane.bind("D", function()
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

| API                                    | Purpose                                                          |
| -------------------------------------- | ---------------------------------------------------------------- |
| `tpane.use(name_or_spec)`              | Load a built-in or git plugin.                                   |
| `tpane.opt`                            | Set tmux options with assignment, like `tpane.opt.mouse = true`. |
| `tpane.append(name, value)`            | Append to tmux options such as `update-environment`.             |
| `tpane.options(table)`                 | Set nested tmux options.                                         |
| `tpane.bind(key, action, opts)`        | Bind a key.                                                      |
| `tpane.unbind(key, opts)`              | Remove a key binding.                                            |
| `tpane.raw(command)`                   | Build an action from raw tmux command text.                      |
| `tpane.pane.*`                         | Pane actions.                                                    |
| `tpane.window.*`                       | Window actions.                                                  |
| `tpane.copy.*`                         | Copy-mode actions.                                               |
| `tpane.key.*`                          | Key helpers such as `tpane.key.prefix()`.                        |
| `tpane.widget(fn)`                     | Create a widget handle.                                          |
| `tpane.widgets.*`                      | Built-in widget handles and factories.                           |
| `tpane.job(opts)`                      | Run shell-backed widget data in the background.                  |
| `tpane.statusline(opts)`               | Configure the tmux statusline.                                   |
| `tpane.theme(name_or_palette[, opts])` | Apply a theme from the `themes` plugin.                          |
| `tpane.tabline(opts)`                  | Configure tmux window tabs.                                      |
| `tpane.register_pane(name, opts)`      | Register a reusable pane definition.                             |
| `tpane.split(target, opts)`            | Split/open a reusable pane.                                      |
| `tpane.toggle(target, opts)`           | Toggle a reusable pane.                                          |
| `tpane.show(target, opts)`             | Show a reusable pane.                                            |
| `tpane.hide(target, opts)`             | Hide a reusable pane.                                            |
| `tpane.expand(target, opts)`           | Zoom or expand a pane.                                           |
| `tpane.workspace(def)`                 | Register a named layout.                                         |
| `tpane.apply_workspace(name)`          | Apply a registered layout once.                                  |
| `tpane.panes()`                        | Return current pane objects.                                     |
| `tpane.find(query)`                    | Find one pane by fields.                                         |
| `tpane.find_all(query)`                | Find all panes by fields.                                        |
| `tpane.kind(def)`                      | Register pane detection.                                         |
| `tpane.state(name, opts)`              | Register state presentation.                                     |
| `tpane.on(event, fn)`                  | Register an event handler.                                       |
| `tpane.store`                          | Persistent Lua key-value store.                                  |
| `tpane.tmux`                           | Low-level tmux helpers.                                          |
| `tpane.fmt`                            | tmux format helpers.                                             |
