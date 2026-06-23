local function state_presentation(state)
  if not state then return {} end
  return tpane.state(state) or {}
end

local function state_segment(state, fallback_glyph)
  local presentation = state_presentation(state)
  if not presentation.color then return nil end
  return { text = presentation.glyph or fallback_glyph or "●", fg = presentation.color }
end

local function hidden_pane(pane)
  return pane.session and pane.session:match("^__tpane%-hidden%-") ~= nil
end

local function compact_pane_segment(pane)
  local presentation = state_presentation(pane.state)
  if hidden_pane(pane) then
    return { text = "○", fg = presentation.color or "default" }
  end
  return state_segment(pane.state, "●") or { text = "○", fg = "default" }
end

local agent_kinds = { pi = true, claude = true, codex = true }

local function is_agent_pane(pane)
  return pane.tag == "agent" or agent_kinds[pane.kind] == true
end

local function agent_window(pane)
  return pane.home or pane.window
end

local function agent_pane_segment(pane)
  if hidden_pane(pane) then return nil end
  if pane.state == nil or pane.state == "idle" or pane.state == "idle_seen" then return nil end
  return compact_pane_segment(pane)
end

local function agent_aggregate_segment(state, count)
  local segment
  if state == "idle" then
    segment = { text = "○", fg = "default" }
  else
    segment = state_segment(state, "●") or { text = "○", fg = "default" }
  end
  segment.text = segment.text .. tostring(count)
  return segment
end

local function agent_aggregate_state(pane)
  if hidden_pane(pane) then return "idle" end
  if pane.state == "approval" then return "approval" end
  if pane.state == "blocked" then return "blocked" end
  if pane.state == "working" then return "working" end
  if pane.state == "done_unseen" then return "done_unseen" end
  return "idle"
end

local function push_agent_pane(parts, pane)
  local segment = agent_pane_segment(pane)
  if segment then
    parts[#parts + 1] = segment
    parts[#parts + 1] = { text = " " .. pane.label }
  else
    parts[#parts + 1] = { text = pane.label }
  end
  parts[#parts + 1] = "  "
end

local function push_agent_group(parts, panes)
  local counts = {}
  for _, pane in ipairs(panes) do
    local state = agent_aggregate_state(pane)
    counts[state] = (counts[state] or 0) + 1
  end
  for _, state in ipairs({ "approval", "blocked", "working", "done_unseen" }) do
    if counts[state] then
      parts[#parts + 1] = agent_aggregate_segment(state, counts[state])
      parts[#parts + 1] = " "
    end
  end
  if parts[#parts] == " " then parts[#parts] = nil end
end

local agent_needs = { blocked = true, approval = true, done_unseen = true }

local function collect_agent_panes(window, needy_only)
  local found = {}
  for _, pane in ipairs(tpane.panes()) do
    if is_agent_pane(pane) and not hidden_pane(pane) and (not window or pane.window == window) then
      if not needy_only or agent_needs[pane.state] then
        found[#found + 1] = pane
      end
    end
  end
  return found
end

tpane.command("agent_next", function()
  local current = tpane.tmux.current_pane()
  local current_window
  for _, pane in ipairs(tpane.panes()) do
    if pane.id == current then
      current_window = pane.window
      break
    end
  end

  local list = collect_agent_panes(current_window, true)
  if #list == 0 then list = collect_agent_panes(current_window, false) end
  if #list == 0 then list = collect_agent_panes(nil, true) end
  if #list == 0 then list = collect_agent_panes(nil, false) end
  if #list == 0 then return nil end

  local idx = 0
  for i, pane in ipairs(list) do
    if pane.id == current then
      idx = i
      break
    end
  end
  tpane.tmux.select(list[(idx % #list) + 1].id)
end)

tpane.widget("agents", function(ctx)
  local active = {}
  local other_groups = {}
  local other_windows = {}
  for _, pane in ipairs(ctx.panes or {}) do
    if is_agent_pane(pane) then
      local window = agent_window(pane)
      if window == ctx.window then
        active[#active + 1] = pane
      else
        if not other_groups[window] then
          other_groups[window] = {}
          other_windows[#other_windows + 1] = window
        end
        other_groups[window][#other_groups[window] + 1] = pane
      end
    end
  end

  local parts = {}
  for _, pane in ipairs(active) do
    push_agent_pane(parts, pane)
  end
  if #parts > 0 then parts[#parts] = nil end

  local aggregate_groups = {}
  for _, window in ipairs(other_windows) do
    local group = {}
    push_agent_group(group, other_groups[window])
    if #group > 0 then aggregate_groups[#aggregate_groups + 1] = group end
  end

  if #parts > 0 and #aggregate_groups > 0 then parts[#parts + 1] = "  |  " end
  for idx, group in ipairs(aggregate_groups) do
    if idx > 1 then parts[#parts + 1] = " | " end
    for _, part in ipairs(group) do
      parts[#parts + 1] = part
    end
  end
  if #parts == 0 then return nil end
  return parts
end)
