
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

tpane.state("blocked", { color = "red", glyph = "●" })
tpane.state("working", { color = "yellow", glyph = "●" })
tpane.state("done_unseen", { color = "blue", glyph = "●" })
tpane.state("idle_seen", { color = "green", glyph = "●" })

tpane.widget("session", function(ctx)
  return { text = "[" .. (ctx.session or "") .. "]" }
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

tpane.widget("companions", function(ctx)
  local parts = {}
  for _, pane in ipairs(ctx.panes or {}) do
    if pane.home then
      local hidden = pane.session and pane.session:match("^__tpane%-hidden%-") ~= nil
      if hidden then
        local presentation = state_presentation(pane.state)
        parts[#parts + 1] = { text = "○", fg = presentation.color or "default" }
      else
        parts[#parts + 1] = state_segment(pane.state, "●")
      end
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

local function show_companion(from, opts)
  local visible = tpane.find(companion_query(from, opts))
  if visible then return visible end

  local hidden = tpane.find { session = "__tpane-hidden-" .. from.window, tag = opts.tag, home = from.window }
    or tpane.find { session = "__pi-hidden-" .. from.window, tag = opts.tag, home = from.window }
  if hidden then
    tpane.tmux.unstash {
      pane = hidden.id,
      target = from.id,
      horizontal = companion_horizontal(opts),
      size = opts.size,
    }
    tpane.tmux.select(hidden.id)
    return hidden
  end

  local pane = tpane.split(from, {
    dir = opts.dir,
    size = opts.size,
    cwd = from.cwd,
    command = opts.command,
    detached = true,
    tag = opts.tag,
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
