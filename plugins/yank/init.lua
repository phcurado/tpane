tpane.bind("y", tpane.copy.copy(), { mode = "copy" })
tpane.bind("Enter", tpane.copy.copy(), { mode = "copy" })

tpane.bind("Y", tpane.raw('set-buffer -w "#{pane_current_path}"'))

tpane.bind(
  "MouseDragEnd1Pane",
  tpane.raw("send-keys -X copy-selection-and-cancel"),
  { mode = "copy" }
)
