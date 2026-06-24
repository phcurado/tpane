local directions = {
  ["C-h"] = { tmux = "L", pane = "left" },
  ["C-j"] = { tmux = "D", pane = "down" },
  ["C-k"] = { tmux = "U", pane = "up" },
  ["C-l"] = { tmux = "R", pane = "right" },
}

for key, direction in pairs(directions) do
  tpane.bind(
    key,
    "if-shell -F '#{m:*vim*,#{pane_current_command}}' 'send-keys " .. key .. "' 'select-pane -" .. direction.tmux .. "'",
    { prefix = false }
  )
  tpane.bind(key, tpane.pane.select(direction.pane), { mode = "copy" })
end
