# Lua API

Castr loads every `*.lua` file under `~/.config/castr`.

## Kinds

```lua
castr.kind { name = "psql", match = "psql" }
```

`match` checks argv tokens in the pane process tree. It is exact: `pi` matches
`pi` or `/usr/bin/pi`, not `pip` or `compile`.

Use `detect` when matching needs code:

```lua
castr.kind {
  name = "server",
  detect = function(p)
    return p:running("node") and p.cwd_basename == "api"
  end,
}
```

Add `state` when a kind should be polled:

```lua
castr.kind {
  name = "worker",
  match = "worker",
  state = function(p)
    if p:var("@castr_push_state") == "blocked" then return "blocked" end
    if p:capture():match("Working") then return "working" end
    return "idle"
  end,
}
```

States: `blocked`, `working`, `done_unseen`, `idle_seen`.

## Panes

Pane fields:

```lua
p.id
p.pid
p.cwd
p.cwd_basename
p.command
p.session
p.window
p.active
p.zoomed
p.kind      -- detected thing running in the pane
p.label     -- rendered text
p.tag       -- user marker for finding panes later
p.home
p.state
```

Pane methods:

```lua
p:running("psql")
p:var("@tmux_var")
p:set { tag = "logs", home = p.window, label = "logs" }
p:capture()
p:proc_tree():list()
p:proc_tree():any(function(proc) return proc.argv:match("--debug") end)
```

## Find panes

```lua
local pane = castr.find{ tag = "logs", window = current.window }
local agents = castr.find_all{ tag = "agent" }
```

Query fields are pane fields. All fields in the query must match.

## Companion panes

Persistent side/bottom panes are one data table plus `toggle` / `expand`:

```lua
castr.register_pane("logs", { dir = "below", size = "25%", command = "zsh" })

castr.bind_key("root", "M-e", function(pane)
  castr.toggle(pane, "logs")
end)

castr.bind_key("root", "M-E", function(pane)
  castr.expand(pane, "logs")
end)
```

`castr.register_pane(name, opts)` defines a reusable pane config. It does not
touch tmux or create/select a pane. The default `tag` and stash `name` are the
same as `name` unless set in `opts`.

`toggle` shows/hides the pane. Hidden panes are stashed so their process keeps
running. `expand` shows the pane and makes it the expanded tmux pane.

`dir`: `below`, `above`, `right`, `left`.

Low-level `castr.split(pane, opts)` still exists and returns a pane handle.

## Commands

```lua
castr.command("hello", function(args)
  return "hi"
end)
```

Table form also works:

```lua
castr.command { name = "hello", handler = function(args) return "hi" end }
```

Then:

```sh
castr hello
```

## Key bindings

```lua
local logs = { tag = "logs", dir = "below", size = "25%", command = "zsh" }

castr.bind_key("root", "M-e", function(pane)
  castr.toggle(pane, logs)
end)
```

You can still bind to commands:

```lua
castr.bind_key("a", { "hello" })
castr.bind_key("Space", { "control" }, { popup = true })
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

`pane` is the pane that invoked the binding. Extra raw CLI args are passed as a
second argument.

## Low-level tmux helpers

Use these when the high-level helpers are not enough:

```lua
castr.tmux.split { target = pane_id, direction = "h", size = "35%", cwd = p.cwd, command = "zsh" }
castr.tmux.unstash { pane = pane_id, target = target_id, horizontal = true, size = "35%" }
castr.tmux.stash { pane = pane_id, window = window_id, cwd = p.cwd, name = "hidden" }
castr.tmux.unzoom(window_id)
castr.tmux.select(pane_id)
castr.tmux.zoom(pane_id) -- tmux resize-pane -Z
castr.tmux.display { target = pane_id, message = "message" }
```

## Compatibility aliases

```lua
castr.register_kind    -- castr.kind
castr.register_command -- castr.command
castr.register_panel   -- castr.panel
```
