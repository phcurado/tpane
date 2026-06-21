# Lua API

Castr loads every `*.lua` file under `~/.config/castr`.

## Kinds

A kind tells castr how to recognize what is running in a pane.

Most kinds only need a process name:

```lua
castr.kind { name = "psql", match = "psql" }
```

This labels a pane as `psql` when any process in that pane is running `psql`.
The match is exact: `pi` matches `pi` or `/usr/bin/pi`, but not `pip` or
`compile`.

Use `detect` when process name is not enough:

```lua
castr.kind {
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
castr.kind {
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

A kind can report state. Castr uses it for border/status/control indicators.

```lua
castr.kind {
  name = "worker",
  match = "worker",
  state = function(pane)
    if pane:var("@castr_push_state") == "blocked" then return "blocked" end
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

Kind callbacks, key handlers, events, and `castr.panes()` use pane objects.

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
pane.tag           -- user tag set by castr
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
local pane = castr.find { tag = "logs", window = current.window }
local agents = castr.find_all { tag = "agent" }
```

All fields in the query must match.

## Reusable panes

Register a pane you want to show/hide later:

```lua
castr.register_pane("logs", {
  dir = "below",
  size = "25%",
  command = "zsh",
})
```

Bind keys to it:

```lua
castr.bind_key("root", "M-e", function(pane)
  castr.toggle(pane, "logs")
end)

castr.bind_key("root", "M-E", function(pane)
  castr.expand(pane, "logs")
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
local logs = castr.split(pane, {
  dir = "below",
  size = "25%",
  command = "zsh",
  tag = "logs",
})
```

It returns a pane handle.

## Key bindings

```lua
castr.bind_key("a", function(pane)
  castr.toggle(pane, "logs")
end)
```

The function receives the pane that invoked the binding.

Bind to a command instead:

```lua
castr.bind_key("a", { "hello" })
castr.bind_key("Space", { "control" }, { popup = true })
```

Options: `popup`, `context`.

## Commands

Use commands when you want a CLI verb:

```lua
castr.command("hello", function(args)
  return "hi " .. (args[1] or "")
end)
```

Then:

```sh
castr hello there
```

## Panels

```lua
castr.panel {
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
castr.on("tick", function() end)
castr.on("pane:new", function(pane) end)
castr.on("pane:focus", function(pane) end)
castr.on("window:close", function(window_id) end)
castr.on("state:change", function(pane_id) end)
```

## Low-level tmux helpers

Use these when the helpers above are not enough:

```lua
castr.tmux.split { target = pane.id, dir = "below", size = "25%", cwd = pane.cwd }
castr.tmux.stash { pane = pane.id, window = pane.window, cwd = pane.cwd, name = "hidden" }
castr.tmux.unstash { pane = hidden.id, target = pane.id, horizontal = true, size = "35%" }
castr.tmux.unzoom(pane.window)
castr.tmux.select(pane.id)
castr.tmux.zoom(pane.id) -- tmux resize-pane -Z
castr.tmux.display { target = pane.id, message = "message" }
```

## Compatibility aliases

```lua
castr.register_kind    -- castr.kind
castr.register_command -- castr.command
castr.register_panel   -- castr.panel
```
