# Quick start

Install tpane:

```sh
curl -fsSL https://raw.githubusercontent.com/phcurado/tpane/main/install.sh | sh
```

Add tpane at the end of `~/.config/tmux/tmux.conf`:

```tmux
run-shell -b 'tpane'
```

Create `~/.config/tmux/tpane/init.lua`:

```lua
tpane.use("sensible")
tpane.use("themes")

tpane.theme("Catppuccin Mocha")

tpane.opt.mouse = true
tpane.opt.mode_keys = "vi"

tpane.bind("h", tpane.pane.select("left"))
tpane.bind("j", tpane.pane.select("down"))
tpane.bind("k", tpane.pane.select("up"))
tpane.bind("l", tpane.pane.select("right"))

local battery = tpane.widgets.battery({ every = "30s" })

tpane.statusline {
  position = "top",
  left = { tpane.widgets.session, tpane.widgets.tabs },
  right = { battery, tpane.widgets.clock, tpane.widgets.date, tpane.widgets.prefix },
}
```

Reload from inside tmux:

```sh
tpane reload
```

Check errors with:

```sh
tpane status
```
