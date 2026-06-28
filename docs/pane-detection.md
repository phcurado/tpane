# Pane detection

Pane detection lets tpane recognize panes by command, directory, output, or
process tree.

Nothing is detected unless you add kind rules yourself or load the built-in pane
detection plugin:

```lua
tpane.use("pane-detection")
```

The built-in plugin also enables pane border titles.

## Kinds

This marks panes running `psql` as database panes:

```lua
tpane.kind { name = "database", match = "psql" }
```

Jump to the database pane later:

```lua
tpane.bind("D", function()
  local db = tpane.find { kind = "database" }
  if db then
    tpane.tmux.select(db.id)
  end
end)
```

Show the current pane label in the status bar:

```lua
local pane_label = tpane.widget(function(ctx)
  return ctx.pane and ctx.pane.label or ""
end)
```

## Match

Use `match` for a command name:

```lua
tpane.kind { name = "editor", match = "nvim" }
```

## Detect

Use `detect` when the rule needs Lua:

```lua
tpane.kind {
  name = "server",
  detect = function(pane)
    return pane:running("node") and pane.cwd:match("/server$") ~= nil
  end,
}
```

## Labels

By default, the label is the kind name. Use `label` to change it:

```lua
tpane.kind {
  name = "editor",
  match = "nvim",
  label = function(pane)
    return "editor " .. pane.cwd_basename
  end,
}
```

## States

A state is a short status attached to a pane.

```lua
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

Show it in the status bar:

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

Add your own state style:

```lua
tpane.state("waiting", { color = "yellow", glyph = "…" })
```

## Pane helpers

```lua
pane:running("psql")
```

Checks whether the pane process tree contains `psql`.

```lua
local text = pane:capture()
```

Reads the visible text in the pane.

```lua
tpane.kind {
  name = "test watcher",
  detect = function(pane)
    return pane:proc_tree():any(function(proc)
      return proc.argv:match("--watch") ~= nil
    end)
  end,
}
```

Checks all processes under the pane.
