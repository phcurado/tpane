
local pane_ref = tpane.pane

tpane._pane_defs = {}
tpane._workspaces = {}
tpane._applied_workspaces = {}
tpane.fmt = {}
tpane.key = {}
tpane.copy = {}
tpane.window = {}
tpane.pane = setmetatable({}, {
  __call = function(_, id) return pane_ref(id) end,
})

local function raw(command)
  return { __tpane_action = "raw", command = command }
end

function tpane.raw(command)
  if type(command) == "table" then command = table.concat(command, " ; ") end
  return raw(command)
end

function tpane.run(command)
  local parts = {}
  if type(command) == "table" then
    for idx, part in ipairs(command) do parts[idx] = part end
  else
    for part in tostring(command):gmatch("%S+") do parts[#parts + 1] = part end
  end
  return { __tpane_action = "run", command = parts }
end

function tpane.key.prefix()
  return raw("send-prefix")
end

local directions = {
  left = "L",
  right = "R",
  up = "U",
  down = "D",
}

function tpane.pane.select(direction)
  return raw("select-pane -" .. assert(directions[direction], "unknown direction"))
end

function tpane.pane.resize(direction, amount)
  return raw("resize-pane -" .. assert(directions[direction], "unknown direction") .. " " .. tostring(amount or 1))
end

function tpane.pane.split(direction, opts)
  opts = opts or {}
  local command = "split-window"
  if direction == "left" then
    command = command .. " -h -b"
  elseif direction == "right" then
    command = command .. " -h"
  elseif direction == "up" then
    command = command .. " -v -b"
  elseif direction == "down" then
    command = command .. " -v"
  else
    error("unknown direction")
  end
  if opts.cwd == "pane" then command = command .. ' -c "#{pane_current_path}"' end
  return raw(command)
end

function tpane.copy.begin(opts)
  opts = opts or {}
  if opts.rectangle then
    return raw("send-keys -X begin-selection \\; send-keys -X rectangle-toggle")
  end
  return raw("send-keys -X begin-selection")
end

function tpane.copy.rectangle()
  return raw("send-keys -X rectangle-toggle")
end

function tpane.copy.copy()
  return raw("send-keys -X copy-selection")
end

function tpane.window.new(opts)
  opts = opts or {}
  local command = "new-window"
  if opts.cwd == "pane" then command = command .. ' -c "#{pane_current_path}"' end
  return raw(command)
end

function tpane.window.previous()
  return raw("previous-window")
end

function tpane.window.next()
  return raw("next-window")
end

function tpane.window.swap(direction)
  if direction == "next" or direction == "right" then
    return raw("swap-window -t +1 ; select-window -t +1")
  elseif direction == "prev" or direction == "left" then
    return raw("swap-window -t -1 ; select-window -t -1")
  end
  error("unknown direction")
end

function tpane.fmt.prefix(on, off)
  return "#{?client_prefix," .. on .. "," .. (off or "") .. "}"
end

function tpane.fmt.when(var, yes, no)
  return "#{?" .. var .. "," .. yes .. "," .. (no or "") .. "}"
end

local function tabline_style(opts, text)
  local style = {}
  for key, value in pairs(opts or {}) do
    style[key] = value
  end
  style.text = text
  return style
end

function tpane.tabline(opts)
  opts = opts or {}
  local label = opts.label or "name"
  local label_format = label
  if label == "cwd" then
    label_format = '#(pwd="#{pane_current_path}"; echo ${pwd####*/})'
  elseif label == "name" then
    label_format = "#W"
  end
  local text = label_format
  if opts.index ~= false then text = "#I:" .. text end
  tpane.options {
    window = {
      status = {
        format = tabline_style(opts.inactive, text),
        current_format = tabline_style(opts.current, text),
      },
    },
  }
end

tpane.state("approval", { color = "yellow", glyph = "" })
tpane.state("blocked", { color = "red", glyph = "" })
tpane.state("working", { color = "yellow", glyph = "" })
tpane.state("done_unseen", { color = "blue", glyph = "" })
tpane.state("idle_seen", { color = "green", glyph = "" })

local function state_presentation(state)
  if not state then return {} end
  return tpane.state(state) or {}
end

local function state_segment(state, fallback_glyph)
  local presentation = state_presentation(state)
  if not presentation.color then return nil end
  return { text = presentation.glyph or fallback_glyph or "●", fg = presentation.color }
end

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
    dir = opts.side or opts.dir or opts.direction,
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
  local side = opts.side or opts.dir or opts.direction
  return side == "right" or side == "left" or side == "h" or side == "horizontal"
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
    dir = opts.side or opts.dir,
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
        dir = pane.side or pane.dir or pane.direction,
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
