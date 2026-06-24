# Lua API

tpane loads top-level `*.lua` files under `~/.config/tpane`.
Plugins load only when referenced with `tpane.use`.
Other Lua files are libraries loaded with `require`.

## Kinds

A kind tells tpane what a pane is and what label to show for it.

```lua
tpane.kind { name = "psql", match = "psql" }
```

When a pane is running `psql`, tpane marks that pane like this:

```lua
pane.kind  -- "psql"
pane.label -- "psql"
```

You can use that in a widget. This status widget shows how many `psql` panes are
running:

```lua
tpane.widget("databases", function(ctx)
  local count = 0
  for _, pane in ipairs(ctx.panes) do
    if pane.kind == "psql" then count = count + 1 end
  end
  if count == 0 then return nil end
  return { text = "db " .. count, fg = "green" }
end)

tpane.statusline { right = { "databases", "clock" } }
```

Now, when any pane in tmux is running `psql`, the statusline shows `db 1`,
`db 2`, and so on. Full paths work too, so `/usr/bin/psql` is treated as
`psql`.

Use `detect` when process name is not enough:

```lua
tpane.kind {
  name = "server",
  detect = function(pane)
    return pane:running("node") and pane.cwd:match("/server$") ~= nil
  end,
}
```

`pane:running("node")` checks the pane's process tree for a `node` process.
`pane.cwd` is the pane's current directory.

You can change the shown label:

```lua
tpane.kind {
  name = "editor",
  match = "my-editor",
  label = function(pane)
    return "editor · " .. pane.cwd_basename
  end,
}
```

## State

A kind can report state. tpane uses it for border/status/control indicators.

```lua
tpane.kind {
  name = "worker",
  match = "worker",
  state = function(pane)
    if pane:var("@tpane_push_state") == "blocked" then return "blocked" end
    if pane:capture():match("Running") then return "working" end
    return "idle"
  end,
}
```

State values are plain strings. Built-in presentations exist for:

```text
approval
blocked
working
done_unseen
idle_seen
```

Declare how custom states render with `tpane.state`:

```lua
tpane.state("approval", { color = "yellow", glyph = "" })
local presentation = tpane.state("approval")
```

`color` is a tmux color. `glyph` is a marker used by Lua renderers such as the
`companions` widget, pane border renderer, or plugins. Detection stays
separate: a kind's `state` function may return any string. Returning `done`
from detection is treated as `done_unseen` until the pane is focused.

Example approval state:

```lua
tpane.state("approval", { color = "yellow", glyph = "" })

tpane.kind {
  name = "reviewer",
  match = "reviewer",
  state = function(pane)
    if pane:capture():match("approval required") then return "approval" end
    if pane:capture():match("running") then return "working" end
    return "idle"
  end,
}

tpane.statusline {
  right = { "companions", "clock" },
}
```

## Pane objects

Kind callbacks, key handlers, events, and `tpane.panes()` use pane objects.

Fields:

```lua
pane.id            -- tmux pane id, like %3
pane.pid           -- root process pid
pane.cwd           -- current directory
pane.cwd_basename  -- last path component of cwd
pane.command       -- tmux pane_current_command
pane.session       -- tmux session name
pane.window        -- tmux window id, like @2
pane.active        -- true if focused
pane.zoomed        -- true if the window is zoomed
pane.kind          -- detected kind
pane.label         -- shown label
pane.tag           -- user tag set by tpane
pane.home          -- user home window for stashed panes
pane.state         -- current state, if any
```

Methods:

```lua
pane:running("psql")
pane:var("@tmux_var")
pane:set { tag = "logs", label = "logs" }
pane:capture()
```

For uncommon process matching, inspect the process tree:

```lua
pane:proc_tree():any(function(proc)
  return proc.argv:match("--watch") ~= nil
end)
```

## Find panes

```lua
local current = tpane.panes()[1]
local logs = tpane.find { tag = "logs", window = current.window }
local all_logs = tpane.find_all { tag = "logs" }
```

All fields in the query must match.

## Reusable panes

Register a pane you want to show/hide later:

```lua
tpane.register_pane("logs", {
  dir = "below",
  size = "25%",
  command = "tail -f logs/app.log",
})
```

Bind keys to it:

```lua
tpane.bind("M-e", function(pane)
  tpane.toggle(pane, "logs")
end, { prefix = false })

tpane.bind("M-E", function(pane)
  tpane.expand(pane, "logs")
end, { prefix = false })
```

`toggle` shows or hides it. Hidden panes are stashed, so the process keeps
running. `expand` shows it and expands it.

Options:

```lua
tag = "logs"              -- defaults to the registered name
name = "logs"             -- stash name, defaults to the registered name
side = "bottom"           -- bottom | top | right | left; dir/direction are accepted aliases
size = "25%"
full = true               -- split across the full window, not only the current pane
anchor = { tag = "editor" } -- optional target pane for split/unstash; defaults to the window's non-companion pane
command = "tail -f logs/app.log" -- command to run in the pane
title = "logs"
label = "logs"
blocked_message = "..."   -- shown instead of hiding a blocked pane
```

## Split directly

Use this when you do not need a registered pane:

```lua
local logs = tpane.split(pane, {
  side = "bottom",
  size = "25%",
  full = true,
  command = "zsh",
  tag = "logs",
})
```

It returns a pane handle.

## tmux.conf equivalents

| tmux.conf                                                | Lua                                                                       |
| -------------------------------------------------------- | ------------------------------------------------------------------------- |
| `set -g mouse on`                                        | `tpane.opt.mouse = true`                                                  |
| `set -g history-limit 5000`                              | `tpane.opt.history_limit = 5000`                                          |
| `set -ga update-environment TERM`                        | `tpane.append("update_environment", "TERM")`                              |
| `unbind C-b`                                             | `tpane.unbind("C-b")`                                                     |
| `bind h select-pane -L`                                  | `tpane.bind("h", tpane.pane.select("left"))`                              |
| `bind -n M-Left resize-pane -L 10`                       | `tpane.bind("M-Left", tpane.pane.resize("left", 10), { prefix = false })` |
| `bind -T copy-mode-vi y send-keys -X copy-selection`     | `tpane.bind("y", tpane.copy.copy(), { mode = "copy" })`                   |
| `bind % split-window -h -c "#{pane_current_path}"`       | `tpane.bind("%", tpane.pane.split("right", { cwd = "pane" }))`            |
| `bind -n C-S-l swap-window -t +1 \; select-window -t +1` | `tpane.bind("C-S-l", tpane.window.swap("next"), { prefix = false })`      |

## Key bindings

```lua
tpane.bind("a", function(pane)
  tpane.toggle(pane, "logs")
end)
```

The function receives the pane that invoked the binding.

Bind to a command instead:

```lua
tpane.bind("a", tpane.run("hello"))
tpane.bind("Space", tpane.run("control"), { popup = true })
```

Options: `popup`, `context`, `prefix`, `mode`, `table`.

By default, bindings use tmux's prefix table. Use `prefix = false` for a
no-prefix binding. Use `mode = "copy"` for copy mode.

```lua
tpane.bind("C-g", function(pane)
  tpane.expand(pane)
end, { prefix = false })

tpane.bind("h", tpane.pane.select("left"))
tpane.bind("M-Left", tpane.pane.resize("left", 10), { prefix = false })
tpane.bind("v", tpane.copy.begin { rectangle = true }, { mode = "copy" })
```

## Commands

Use commands when you want a CLI verb:

```lua
tpane.command("hello", function(args)
  return "hi " .. (args[1] or "")
end)
```

Then:

```sh
tpane run hello there
```

## Panels

```lua
tpane.panel {
  id = "workspace",
  title = "Workspace",
  cards = function()
    return {
      { title = "logs", tag = "key", enter = tpane.run("hello") },
    }
  end,
}
```

## Modules and plugins

Use Lua's `require` for shared config code:

```lua
local helper = require("lib.helper") -- ~/.config/tpane/lib/helper.lua
```

Installed plugins live under `~/.local/share/tpane/plugins/<name>`. They are not
auto-loaded. Reference them from config with `tpane.use`:

```lua
tpane.use("foo", { repo = "https://github.com/example/tpane-plugin.git", branch = "main" })
tpane.use("theme", { repo = "https://github.com/example/theme.git", tag = "v1.2.0" })
tpane.use("tool", { repo = "https://github.com/example/mono.git", rev = "abc123", path = "plugins/tool" })
```

`repo` is the git repository URL (`url` also works). `branch`, `tag`, and `rev`
are mutually exclusive. `path` is relative to the plugin repository and is useful
for monorepos. If `repo` is set and the plugin is missing, tpane installs it
before loading it.

Plugin commands are for maintenance; installation normally comes from `tpane.use`:

```sh
tpane plugin status
tpane plugin sync # install/update plugins referenced by config
tpane plugin update foo
tpane plugin update
tpane plugin clean
tpane plugin list
tpane plugin remove foo
```

## Status line

Define widgets and compose tmux `status-left` / `status-right` from Lua:

```lua
tpane.widget("cwd", function(ctx)
  return { text = ctx.pane and ctx.pane.cwd_basename or "", fg = "green" }
end)

tpane.statusline {
  position = "bottom",
  interval = 1,
  left = { "session" },
  right = { "cwd", "clock" },
  separator = "  ",
}
```

Widget functions receive a context table:

```lua
ctx.session  -- current client session
ctx.window   -- current client window id
ctx.pane     -- current client pane object, or nil when no panes exist
ctx.panes    -- all pane objects
```

Widget functions return a raw string, a styled table with `text`, an array of
strings/styled tables, or `nil` to hide the segment. Raw strings may include tmux
formats and styles. Table style keys mirror tmux attributes: `fg`, `bg`, `bold`,
`dim`, `italics`, `blink`, `reverse`, `hidden`, `strikethrough`, `underscore`,
`align`, and `fill`.

Built-in widgets: `session`, `clock`, `companions`. Raw tmux format strings are
also supported.

Use `tpane.tabline` for the common window-status format without hand-writing
nested tmux options:

```lua
tpane.tabline {
  label = "cwd", -- "cwd", "name", or any raw tmux format string
  inactive = { fg = "#777777" },
  current = { fg = "#8caaee", bold = true },
}
```

## Tmux options

Set options directly:

```lua
tpane.opt.mouse = true
tpane.opt.history_limit = 5000
tpane.opt.mode_keys = "vi"
```

Or set a batch with nested keys. Underscores become dashes:

```lua
tpane.options {
  status = {
    style = { bg = "default" },
    left_length = 120,
  },
  pane = {
    border = {
      lines = "heavy",
      style = { fg = "#51576d" },
    },
    active = {
      border = {
        style = { fg = "#8caaee" },
      },
    },
  },
}
```

Nested keys become tmux option names by joining them with `-`, so
`status.left_length` sets `status-left-length`.

Style options accept Lua style tables:

```lua
tpane.options {
  status = { style = { bg = "default" } }, -- status-style bg=default
}
```

Format options can be plain strings, or styled tables with `text`:

```lua
tpane.options {
  window = {
    status = {
      current_format = { text = "#I:#W", fg = "blue", bold = true },
    },
  },
}
```

Use a literal option name only when nesting would be awkward:

```lua
tpane.options {
  ["some-tmux-option"] = "value",
}
```

## Format helpers

Use `tpane.fmt` for tmux conditionals that do not have Lua equivalents:

```lua
tpane.fmt.prefix("", "")
tpane.fmt.when("window_zoomed_flag", "Z", "")
```

## Public API surface

The intended public Lua surface is: `tpane.use`, `tpane.kind`, `tpane.state`,
`tpane.widget`, `tpane.statusline`, `tpane.tabline`, `tpane.options`,
`tpane.on`, `tpane.command`, `tpane.panel`, `tpane.bind`, `tpane.panes`,
`tpane.find`, `tpane.find_all`, pane objects/handles, reusable pane helpers
(`register_pane`, `split`, `toggle`, `show`, `hide`, `expand`), `tpane.tmux`,
`tpane.fmt`, and `tpane.store`.

## Persistent store

`tpane.store` is a small JSON-backed scratch store for plugins.

```lua
tpane.store.set("counter", 1)
local value = tpane.store.get("counter")
tpane.store.delete("counter")
```

Values may be strings, numbers, booleans, tables, or nil.

## Workspaces

Declare reusable layouts in Lua:

```lua
tpane.workspace {
  name = "dev",
  windows = {
    { name = "app", command = "zsh" },
    { name = "logs", panes = { { dir = "below", size = "30%", command = "tail -f app.log" } } },
  },
}

tpane.command("dev", function()
  tpane.apply_workspace("dev")
end)
```

## Events

```lua
tpane.on("tick", function() end)
tpane.on("pane:new", function(pane) end)
tpane.on("pane:focus", function(pane) end)
tpane.on("window:close", function(window_id) end)
tpane.on("state:change", function(pane_id) end)
```

## Low-level tmux helpers

Use these when the helpers above are not enough:

```lua
local window = tpane.tmux.new_window { name = "logs", cwd = pane.cwd, command = "zsh" }
tpane.tmux.select_window(window)
tpane.tmux.send_keys { target = pane.id, keys = "npm test", enter = true }
tpane.tmux.split { target = pane.id, dir = "below", size = "25%", cwd = pane.cwd }
tpane.tmux.stash { pane = pane.id, window = pane.window, cwd = pane.cwd, name = "hidden" }
tpane.tmux.unstash { pane = hidden.id, target = pane.id, horizontal = true, size = "35%" }
tpane.tmux.unzoom(pane.window)
tpane.tmux.select(pane.id)
tpane.tmux.zoom(pane.id) -- tmux resize-pane -Z
tpane.tmux.display { target = pane.id, message = "message" }
```

## Compatibility aliases

```lua
tpane.register_kind    -- tpane.kind
tpane.register_command -- tpane.command
tpane.register_panel   -- tpane.panel
```
