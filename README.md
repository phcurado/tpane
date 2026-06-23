# tpane

Extend your tmux configuration with Lua.

## Example

Configure your statusbar in Lua:

```lua
tpane.widget("project", function(ctx)
  return ctx.pane and ctx.pane.cwd_basename or ""
end)

tpane.statusline {
  left = { "session", "project" },
  right = { "clock" },
}
```

Set tmux options with Lua tables when you want styling:

```lua
tpane.options {
  status = { style = { bg = "default" } },
  pane = { border = { style = { fg = "#51576d" } } },
}
```

Or add keybinds and work with complex flows. Say you want a logs pane below the current pane. You want one key to show or hide it without killing the shell inside it, and another key to expand it.

```lua
-- ~/.config/tpane/init.lua
tpane.register_pane("logs", {
  side = "bottom",
  size = "25%",
  command = "tail -f logs/app.log",
})

tpane.bind_key("M-e", function(pane)
  tpane.toggle(pane, "logs")
end, { prefix = false })

tpane.bind_key("M-E", function(pane)
  tpane.expand(pane, "logs")
end, { prefix = false })
```

`M-e` shows or hides a pane running `tail -f logs/app.log` below the current pane.
When hidden, the pane is stashed, so the process inside keeps running.

`M-E` expands the logs pane. Press it again to return to the normal layout.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/phcurado/tpane/main/install.sh | sh
```

From source:

```sh
cargo install --path . --locked --force
```

## tmux

Start it from `tmux.conf`:

```tmux
run-shell -b 'tpane'
```

## Config

tpane loads top-level Lua files and plugin entrypoints under:

```text
~/.config/tpane
```

Full Lua reference: [`docs/lua-api.md`](docs/lua-api.md).

## CLI

```sh
tpane          # start or reload the daemon from inside tmux
tpane status   # show load/runtime errors
tpane reload   # reload Lua config
tpane refresh  # reload and rescan panes
tpane doctor   # inspect hidden panes/sessions
tpane run NAME # run a Lua command
```

Run `tpane --help` for everything else.

## Plugins

```sh
tpane plugin add https://github.com/example/tpane-plugin.git
tpane plugin list
tpane plugin remove tpane-plugin
```
