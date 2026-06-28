# tpane

<p align="center">
  <img src="/logo.png" alt="tpane logo" width="360">
</p>

tpane lets you write tmux configuration in Lua. It comes with plugins, themes,
status bar widgets, and helpers for key bindings, panes, windows, and common tmux
settings.

The docs in this site match the current `main` branch. For release notes and
version history, see the [changelog](changelog.md).

## Demo

<video controls muted loop playsinline src="https://github.com/user-attachments/assets/2e92141d-1e6e-407e-903e-29453cbf95ff" style="width: 100%; border-radius: 8px;"></video>

## Start here

- [Quick start](quick-start.md): install tpane and create your first Lua config.
- [Install](install.md): install script, cargo, mise, and source install.
- [Configuration](configuration.md): config files, options, key bindings, and raw tmux commands.
- [Status bar](status-bar.md): widgets, jobs, rows, and window tabs.
- [Plugins](plugins.md): built-in plugins and git plugins.
- [Reusable panes](reusable-panes.md): panes you can toggle without stopping their process.
- [Pane detection](pane-detection.md): recognize panes by command, directory, output, or process tree.
- [Lua API reference](lua-api.md): full API reference.
- [Changelog](changelog.md): release notes and version history.

If you already have tpane installed, `tpane status` shows config errors and
`tpane reload` reloads your Lua config from inside tmux.
