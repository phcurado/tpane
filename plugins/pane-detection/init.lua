tpane.kind {
  name = "pane",
  detect = function(_p)
    return true
  end,
  label = function(p)
    return p.command
  end,
}

tpane.options {
  pane = {
    border = {
      status = "top",
      format = "#{@tpane_border}",
    },
  },
}

local function state_segment(state, fallback_glyph)
  if not state or state == "" then return nil end
  local presentation = tpane.state(state) or {}
  if not presentation.color then return nil end
  return { text = presentation.glyph or fallback_glyph or "●", fg = presentation.color }
end

tpane.pane_border(function(pane)
  local parts = {}
  local state = state_segment(pane.state, "●")
  if state then
    state.text = state.text .. " "
    parts[#parts + 1] = state
  end
  parts[#parts + 1] = { text = pane.label or pane.command or "", fg = "yellow" }
  return parts
end)
