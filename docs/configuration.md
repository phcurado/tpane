# Configuration

## Config files

tpane loads top-level Lua files from:

```text
~/.config/tmux/tpane
```

Set `TPANE_CONFIG_DIR` to use another directory.

Files in subdirectories are not loaded automatically. Use Lua `require` for
shared modules:

```lua
local colors = require("theme.colors") -- ~/.config/tmux/tpane/theme/colors.lua
```

Reload from inside tmux:

```sh
tpane reload
```

Check errors:

```sh
tpane status
```

## Minimal tmux.conf

```text
set -g default-terminal "xterm-256color"
set -as terminal-features ",xterm-256color:RGB"

set -g base-index 1
set -g pane-base-index 1

unbind C-b
set -g prefix C-a
bind C-a send-prefix

run-shell -b 'tpane'
```

## Options

Use `tpane.opt` for normal tmux options:

```lua
tpane.opt.mouse = true
tpane.opt.history_limit = 5000
tpane.opt.mode_keys = "vi"
tpane.opt.renumber_windows = true
tpane.opt.escape_time = 0
```

Use `tpane.append` when you would use `set -ga` in tmux:

```lua
tpane.append("update_environment", "TERM")
tpane.append("update_environment", "TERM_PROGRAM")
```

`tpane.options` is the table form:

```lua
tpane.options {
  mouse = true,
  mode_keys = "vi",
}
```

## Key bindings

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

Split panes:

```lua
tpane.bind("%", tpane.pane.split("right", { cwd = "pane" }))
tpane.bind('"', tpane.pane.split("down", { cwd = "pane" }))
```

`cwd = "pane"` is tpane shorthand for tmux's `-c "#{pane_current_path}"`.

A binding can also run Lua:

```lua
tpane.bind("L", function(pane)
  tpane.toggle(pane, "logs")
end)
```

Use raw tmux commands when there is no helper:

```lua
tpane.bind("R", "source-file ~/.config/tmux/tmux.conf ; display 'reloaded'")
```

Remove a binding:

```lua
tpane.unbind("C-b")
```

## Copy mode

```lua
tpane.bind("[", "copy-mode")
tpane.bind("v", tpane.copy.begin(), { mode = "copy" })
tpane.bind("r", tpane.copy.rectangle(), { mode = "copy" })
tpane.bind("y", tpane.copy.copy(), { mode = "copy" })
```

`mode = "copy"` maps to tmux's `copy-mode-vi` table.
