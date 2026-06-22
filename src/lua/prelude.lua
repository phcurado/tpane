
tpane._pane_defs = {}
tpane._workspaces = {}
tpane._applied_workspaces = {}
tpane.fmt = {}

function tpane.fmt.prefix(on, off)
  return "#{?client_prefix," .. on .. "," .. (off or "") .. "}"
end

function tpane.fmt.when(var, yes, no)
  return "#{?" .. var .. "," .. yes .. "," .. (no or "") .. "}"
end

tpane.state("approval", { color = "yellow", glyph = "" })
tpane.state("blocked", { color = "red", glyph = "" })
tpane.state("working", { color = "yellow", glyph = "" })
tpane.state("done_unseen", { color = "blue", glyph = "" })
tpane.state("idle_seen", { color = "green", glyph = "" })

tpane.widget("session", function()
  return "[#{client_session}] "
end)

tpane.widget("clock", function()
  return os.date("%H:%M")
end)

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

tpane.widget("companions", function(ctx)
  local parts = {}
  for _, pane in ipairs(ctx.panes or {}) do
    if pane.home then
      parts[#parts + 1] = compact_pane_segment(pane)
      parts[#parts + 1] = { text = " " .. pane.label }
      parts[#parts + 1] = "  "
    end
  end
  if #parts == 0 then return nil end
  parts[#parts] = nil
  return parts
end)

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

function tpane.register_pane(name, opts)
  opts.tag = opts.tag or name
  opts.name = opts.name or name
  tpane._pane_defs[name] = opts
  return opts
end

local function pane_opts(opts)
  if type(opts) == "string" then return tpane._pane_defs[opts] end
  return opts
end

function tpane.find(query)
  for _, pane in ipairs(tpane.panes()) do
    local ok = true
    for key, expected in pairs(query) do
      if pane[key] ~= expected then
        ok = false
        break
      end
    end
    if ok then return pane end
  end
end

function tpane.find_all(query)
  local found = {}
  for _, pane in ipairs(tpane.panes()) do
    local ok = true
    for key, expected in pairs(query) do
      if pane[key] ~= expected then
        ok = false
        break
      end
    end
    if ok then found[#found + 1] = pane end
  end
  return found
end

function tpane.resolve(target)
  if type(target) == "string" then return target end
  if target and target.id then return target.id end
  local pane = tpane.find(target)
  return pane and pane.id
end

function tpane.split(pane, opts)
  local id = tpane.tmux.split {
    target = tpane.resolve(pane),
    dir = opts.dir or opts.direction,
    size = opts.size,
    cwd = opts.cwd,
    command = opts.command,
    detached = opts.detached,
    full = opts.full,
  }
  local created = tpane.pane(id)
  if opts.tag then created:set { tag = opts.tag } end
  return created
end

local function companion_query(from, opts)
  return { tag = opts.tag, window = from.window, home = from.window }
end

local function companion_horizontal(opts)
  return opts.dir == "right" or opts.dir == "left" or opts.dir == "h" or opts.dir == "horizontal"
end

local function find_anchor_query(from, query)
  local scoped = { window = from.window }
  for key, value in pairs(query) do
    scoped[key] = value
  end
  scoped.window = from.window
  return tpane.find(scoped)
end

local function default_anchor(from)
  for _, pane in ipairs(tpane.panes()) do
    if pane.window == from.window and not pane.home then return pane end
  end
  return from
end

local function resolve_anchor(from, anchor)
  if anchor == nil then return default_anchor(from) end
  if type(anchor) == "table" then return find_anchor_query(from, anchor) or from end
  if type(anchor) == "function" then
    local resolved = anchor(from)
    local id = tpane.resolve(resolved)
    if id then return tpane.find { id = id } or tpane.pane(id) end
    return from
  end
  error("anchor must be a table or function")
end

local function show_companion(from, opts)
  local visible = tpane.find(companion_query(from, opts))
  if visible then return visible end

  local anchor = resolve_anchor(from, opts.anchor)
  local hidden = tpane.find { session = "__tpane-hidden-" .. from.window, tag = opts.tag, home = from.window }
    or tpane.find { session = "__pi-hidden-" .. from.window, tag = opts.tag, home = from.window }
  if hidden then
    tpane.tmux.unstash {
      pane = hidden.id,
      target = anchor.id,
      horizontal = companion_horizontal(opts),
      size = opts.size,
      full = opts.full,
    }
    tpane.tmux.select(hidden.id)
    return hidden
  end

  local pane = tpane.split(anchor, {
    dir = opts.dir,
    size = opts.size,
    cwd = anchor.cwd or from.cwd,
    command = opts.command,
    detached = true,
    tag = opts.tag,
    full = opts.full,
  })
  pane:set { home = from.window, title = opts.title, label = opts.label }
  tpane.tmux.select(pane.id)
  return pane
end

local raw_toggle = function(target)
  local id = tpane.resolve(target)
  if not id then return false end
  tpane.tmux.zoom(id)
  return true
end

function tpane.toggle(target, opts)
  if not opts then return raw_toggle(target) end
  opts = pane_opts(opts)
  if not opts then return false end

  local visible = tpane.find(companion_query(target, opts))
  if not visible then
    show_companion(target, opts)
    return true
  end

  if visible.state == "blocked" and opts.blocked_message then
    tpane.tmux.display { target = visible.id, message = opts.blocked_message }
    return false
  end

  tpane.tmux.stash {
    pane = visible.id,
    window = target.window,
    cwd = target.cwd,
    name = opts.name or opts.tag,
  }
  return true
end

function tpane.workspace(def)
  tpane._workspaces[def.name] = def
  return def
end

function tpane.apply_workspace(name)
  local workspace = tpane._workspaces[name]
  if not workspace then return false end
  if tpane._applied_workspaces[name] then return true end

  for _, window in ipairs(workspace.windows or {}) do
    local target = tpane.tmux.new_window {
      name = window.name,
      cwd = window.cwd,
      command = window.command,
    }
    for _, pane in ipairs(window.panes or {}) do
      local created = tpane.tmux.split {
        target = target,
        dir = pane.dir or pane.direction,
        size = pane.size,
        cwd = pane.cwd or window.cwd,
        command = pane.command,
        detached = pane.detached,
      }
      if pane.tag or pane.label or pane.title then
        tpane.pane(created):set { tag = pane.tag, label = pane.label, title = pane.title }
      end
    end
  end

  tpane._applied_workspaces[name] = true
  return true
end

function tpane.expand(target, opts)
  if opts then
    opts = pane_opts(opts)
    if not opts then return false end
    target = show_companion(target, opts)
  end

  local id = tpane.resolve(target)
  if not id then return false end

  local window = tpane.tmux.window_id(id)
  if tpane.tmux.is_zoomed(window) and tpane.tmux.active_pane(window) == id then
    tpane.tmux.unzoom(window)
    return true
  end

  tpane.tmux.unzoom(window)
  tpane.tmux.select(id)
  tpane.tmux.zoom(id)
  return true
end
