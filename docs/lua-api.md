# Lua API

tpane loads every `*.lua` file under `~/.config/tpane`.

## Kinds

A kind tells tpane how to recognize what is running in a pane.

Most kinds only need a process name:

```lua
tpane.kind { name = "psql", match = "psql" }
```

This labels a pane as `psql` when any process in that pane is running `psql`.
The match is exact: `pi` matches `pi` or `/usr/bin/pi`, but not `pip` or
`compile`.

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
  name = "nvim",
  match = "nvim",
  label = function(pane)
    return "nvim · " .. pane.cwd_basename
  end,
}
```

`cwd_basename` is the last part of `cwd`. For `/home/me/project`, it is
`project`.

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

State values:

```text
blocked
working
done_unseen
idle_seen
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
local pane = tpane.find { tag = "logs", window = current.window }
local agents = tpane.find_all { tag = "agent" }
```

All fields in the query must match.

## Reusable panes

Register a pane you want to show/hide later:

```lua
tpane.register_pane("logs", {
  dir = "below",
  size = "25%",
  command = "zsh",
})
```

Bind keys to it:

```lua
tpane.bind_key("root", "M-e", function(pane)
  tpane.toggle(pane, "logs")
end)

tpane.bind_key("root", "M-E", function(pane)
  tpane.expand(pane, "logs")
end)
```

`toggle` shows or hides it. Hidden panes are stashed, so the process keeps
running. `expand` shows it and expands it.

Options:

```lua
tag = "logs"              -- defaults to the registered name
name = "logs"             -- stash name, defaults to the registered name
dir = "below"             -- below | above | right | left
size = "25%"
command = "zsh"
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

Options: `popup`, `context`.

## Commands

Use commands when you want a CLI verb:

```lua
tpane.command("hello", function(args)
  return "hi " .. (args[1] or "")
end)
```

Then:

```sh
tpane hello there
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
