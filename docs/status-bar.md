# Status bar

`tpane.statusline` defines the tmux status bar.

```lua
tpane.statusline {
  position = "top",
  left = { tpane.widgets.session, tpane.widgets.tabs },
  right = { tpane.widgets.clock },
}
```

## Built-in widgets

| Widget                        | Description                                                 |
| ----------------------------- | ----------------------------------------------------------- |
| `tpane.widgets.session`       | Current tmux session.                                       |
| `tpane.widgets.host`          | Hostname from tmux.                                         |
| `tpane.widgets.clock`         | Current time, like `14:30`.                                 |
| `tpane.widgets.date`          | Current date, like `Jun 25`.                                |
| `tpane.widgets.prefix`        | Shows when the tmux prefix key is active.                   |
| `tpane.widgets.tabs`          | tmux window list.                                           |
| `tpane.widgets.cpu(opts)`     | CPU usage. Works on Linux and macOS.                        |
| `tpane.widgets.memory(opts)`  | Used memory. Works on Linux and macOS.                      |
| `tpane.widgets.battery(opts)` | Battery status with icons. Works on Linux and macOS.        |
| `tpane.widgets.player(opts)`  | Current playing track. Uses `playerctl`, Music, or Spotify. |

Widgets with `opts` start background work. Call them once and reuse the result:

```lua
local cpu = tpane.widgets.cpu({ every = "2s" })
local memory = tpane.widgets.memory({ every = "5s" })
local battery = tpane.widgets.battery({ every = "30s" })

tpane.statusline {
  right = { cpu, memory, battery, tpane.widgets.clock },
}
```

## Custom widgets

```lua
local cwd = tpane.widget(function(ctx)
  return ctx.pane and ctx.pane.cwd_basename or ""
end)

tpane.statusline {
  left = { tpane.widgets.session, cwd },
  right = { tpane.widgets.clock },
}
```

`ctx` has the current tmux state:

| Field         | Description                      |
| ------------- | -------------------------------- |
| `ctx.session` | Current session name.            |
| `ctx.window`  | Current window id, like `@2`.    |
| `ctx.pane`    | Current pane object, or `nil`.   |
| `ctx.panes`   | All pane objects known to tpane. |

A widget can return text:

```lua
local server = tpane.widget(function()
  return "server"
end)
```

Or styled text:

```lua
local status = tpane.widget(function()
  return { text = "ok", fg = "green", bold = true }
end)
```

## Shell commands

Use `tpane.job` for status bar data that comes from a shell command:

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

## Multiline status bar

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

```lua
tpane.tabline {
  label = "cwd",
  inactive = { fg = "#777777" },
  current = { fg = "#8caaee", bold = true },
}
```
