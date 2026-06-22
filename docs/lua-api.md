# Lua API

tpane loads top-level `*.lua` files under `~/.config/tpane` and `plugins/*/init.lua`.
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
built-in `agents`/`companions` widgets and pane border renderer. Detection stays
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
tpane.bind_key("M-e", function(pane)
  tpane.toggle(pane, "logs")
end, { prefix = false })

tpane.bind_key("M-E", function(pane)
  tpane.expand(pane, "logs")
end, { prefix = false })
```

`toggle` shows or hides it. Hidden panes are stashed, so the process keeps
running. `expand` shows it and expands it.

Options:

```lua
tag = "logs"              -- defaults to the registered name
name = "logs"             -- stash name, defaults to the registered name
dir = "below"             -- below | above | right | left
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
  dir = "below",
  size = "25%",
  full = true,
  command = "zsh",
  tag = "logs",
})
```

It returns a pane handle.

## Key bindings

```lua
tpane.bind_key("a", function(pane)
  tpane.toggle(pane, "logs")
end)
```

The function receives the pane that invoked the binding.

Bind to a command instead:

```lua
tpane.bind_key("a", { "hello" })
tpane.bind_key("Space", { "control" }, { popup = true })
```

Options: `popup`, `context`, `prefix`, `table`.

By default, bindings use tmux's prefix table (`bind-key a`). Use
`prefix = false` for a no-prefix binding (`bind-key -n C-g`), or `table` for a
named tmux table (`bind-key -T copy-mode-vi v`):

```lua
tpane.bind_key("C-g", function(pane)
  tpane.expand(pane)
end, { prefix = false })

tpane.bind_key("v", { "copy" }, { table = "copy-mode-vi" })
```

Do not use `tpane.bind_key` for keys you press repeatedly, such as pane
movement or resize. Those should stay as plain tmux bindings because
`tpane.bind_key` starts a `tpane run ...` process on each keypress.

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
      { title = "logs", tag = "key", enter = { "hello" } },
    }
  end,
}
```

## Modules

Use Lua's `require` for shared plugin code:

```lua
local helper = require("lib.helper") -- ~/.config/tpane/lib/helper.lua
```

Plugin modules can live under `plugins/<name>/` and be required as
`require("<name>.module")`.

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

Built-in widgets: `session`, `clock`, `agents`, `companions`. `agents` shows
agent panes in the current window by label and groups active/attention states in
other windows as counts. Raw tmux format strings are also supported.

## Tmux options

Set static tmux options from Lua with nested keys. Underscores become dashes:

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
