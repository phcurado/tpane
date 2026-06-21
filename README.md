# tpane

tpane is a small tmux daemon configured with Lua.

It runs next to a tmux server, labels panes by what is running in them, and lets
you describe pane workflows in Lua instead of shell scripts and `tmux.conf` glue.

## Example

Say you want a logs pane below the current pane. You want one key to show or hide
it without killing the shell inside it, and another key to expand it.

```lua
-- ~/.config/tpane/init.lua
tpane.register_pane("logs", {
  dir = "below",
  size = "25%",
  command = "zsh",
})

tpane.bind_key("root", "M-e", function(pane)
  tpane.toggle(pane, "logs")
end)

tpane.bind_key("root", "M-E", function(pane)
  tpane.expand(pane, "logs")
end)
```

`M-e` shows or hides the logs pane below the current pane. It uses 25% of the
window. When hidden, the pane is stashed, so the process inside keeps running.

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

Optional status segment:

```tmux
set -g status-right '#{@tpane_status}'
```

## Config

tpane loads every Lua file under:

```text
~/.config/tpane
```

A basic kind looks like this:

```lua
tpane.kind { name = "psql", match = "psql" }
```

Full Lua reference: [`docs/lua-api.md`](docs/lua-api.md).

## CLI

```sh
tpane          # start or reload the daemon for the current tmux server
tpane status   # show load/runtime errors
tpane reload   # reload Lua config
tpane refresh  # reload and rescan panes
tpane doctor   # inspect hidden panes/sessions
```

Run `tpane --help` for everything else.
