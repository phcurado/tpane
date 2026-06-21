tpane.kind {
  name = "pane",
  detect = function(_p)
    return true
  end,
  label = function(p)
    return p.command
  end,
}
