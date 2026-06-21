# castr

castr is a small tmux daemon configured with Lua.

It runs next to a tmux server, looks at each pane, and keeps a little bit of
state about it: what is running there, what label to show, whether it is blocked
or working, and any tag you gave it.

The point is to move tmux workflow out of shell scripts and `tmux.conf`, using only the lua language to define the logic of your panes.

## Example

Say you want a logs pane below the main pane. You want one key to show/hide it
without killing the process, and another key to expand it.

```lua
# ~/.config/castr/init.lua
castr.register_pane("logs", {
  dir = "below",
  size = "25%",
  command = "zsh",
})

castr.bind_key("root", "M-e", function(pane)
  castr.toggle(pane, "logs")
end)

castr.bind_key("root", "M-E", function(pane)
  castr.expand(pane, "logs")
end)
```

`Super + e` will show/hide and hide a panel below your main panel, with 25% size.
`Super + E` will expand the panel to full screen in your terminal, and pressing again the same key will make it go back to the previous state (25% size).

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/phcurado/castr/main/install.sh | sh
```

From source:

```sh
cargo install --path . --locked --force
```

## tmux

Start it from `tmux.conf`:

```tmux
run-shell -b 'castr'
```

Optional status segment:

```tmux
set -g status-right '#{@castr_status}'
```

## Config

castr loads every Lua file under:

```text
~/.config/castr
```

A basic kind looks like this:

```lua
castr.kind { name = "psql", match = "psql" }
```

Full Lua reference: [`docs/lua-api.md`](docs/lua-api.md).

## CLI

```sh
castr          # start or reload the daemon for the current tmux server
castr status   # show load/runtime errors
castr reload   # reload Lua config
castr refresh  # reload and rescan panes
castr doctor   # inspect hidden panes/sessions
```

Run `castr --help` for everything else.
