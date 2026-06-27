# Reusable panes

Reusable panes can be shown and hidden without stopping the process running
inside them.

```lua
tpane.register_pane("logs", {
  side = "bottom",
  size = "25%",
  command = "tail -f logs/app.log",
})
```

Toggle it from a key binding:

```lua
tpane.bind("L", function(pane)
  tpane.toggle(pane, "logs")
end)
```

Show it and zoom the layout around it:

```lua
tpane.bind("M-L", function(pane)
  tpane.expand(pane, "logs")
end, { prefix = false })
```

Common options:

| Option    | Description                                          |
| --------- | ---------------------------------------------------- |
| `side`    | Where to split: `bottom`, `top`, `right`, or `left`. |
| `size`    | tmux split size, like `25%`.                         |
| `command` | Command to run when creating the pane.               |
| `tag`     | Pane tag. Defaults to the registered name.           |
| `label`   | tpane label for the pane.                            |

Use `tpane.split` for a one-off split:

```lua
tpane.bind("S", function(pane)
  tpane.split(pane, {
    side = "bottom",
    size = "25%",
    command = "zsh",
  })
end)
```
