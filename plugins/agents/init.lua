tpane.agents = tpane.agents or {}

tpane.state("waiting", { color = "yellow", glyph = "" })

local definitions = {}

local function default_state(pane)
	local pushed = pane:var("@tpane_push_state")
	if pushed and pushed ~= "" then
		return pushed
	end
	return "idle"
end

local function register(def)
	definitions[def.name] = def

	tpane.kind({
		name = def.name,
		tag = "agent",
		detect = function(pane)
			for _, command in ipairs(def.commands or {}) do
				if pane.command == command or pane:running(command) then
					return true
				end
			end
			return false
		end,
		label = function()
			return def.label or def.name
		end,
		state = def.state or default_state,
	})
end

function tpane.agents.register(def)
	assert(def.name, "agent requires name")
	def.label = def.label or def.name
	def.commands = def.commands or { def.name }
	register(def)
end

register({ name = "claude", label = "claude", icon = "✻", commands = { "claude" } })
register({ name = "codex", label = "codex", icon = "◇", commands = { "codex" } })
register({ name = "opencode", label = "opencode", icon = "◆", commands = { "opencode" } })
register({ name = "gemini", label = "gemini", icon = "✦", commands = { "gemini" } })
register({ name = "pi", label = "pi", icon = "π", commands = { "pi" } })

local priority = {
	blocked = 1,
	waiting = 1,
	approval = 1,
	done_unseen = 2,
	working = 3,
	idle = 4,
	idle_seen = 4,
}

local symbols = {
	blocked = "",
	waiting = "",
	approval = "",
	done_unseen = "",
	idle = "󰒲",
	idle_seen = "󰒲",
}

local colors = {
	blocked = { fg = "#11111b", bg = "#f38ba8" },
	waiting = { fg = "#f38ba8" },
	approval = { fg = "#11111b", bg = "#f38ba8" },
	done_unseen = { fg = "#a6e3a1" },
	working = { fg = "#f9e2af" },
	idle = { fg = "#6c7086" },
	idle_seen = { fg = "#6c7086" },
}

local function state_symbol(state)
	if state == "working" then
		return ""
	end
	return symbols[state]
end

local function window_style(state)
	local color = colors[state]
	if not color or not color.bg then
		return ""
	end
	return "#[fg=" .. color.fg .. ",bg=" .. color.bg .. "]"
end

local function window_indicator(state, count)
	local symbol = state_symbol(state)
	if not symbol then
		return ""
	end
	if count > 1 then
		symbol = symbol .. tostring(count)
	end
	local color = colors[state]
	if color and not color.bg then
		return "#[fg=" .. color.fg .. "] " .. symbol .. " "
	end
	return " " .. symbol .. " "
end

function tpane.agents._tab_state(state, count)
	return {
		indicator = window_indicator(state, count or 1),
		style = window_style(state),
	}
end

local function context_for(pane)
	if pane.cwd_basename and pane.cwd_basename ~= "" then
		return pane.cwd_basename
	end
	if pane.session and pane.session ~= "" then
		return pane.session
	end
	return pane.window or ""
end

local function agent_for(pane)
	if pane.tag ~= "agent" then
		return nil
	end
	return definitions[pane.kind] or definitions[pane.label] or { name = pane.kind, label = pane.label, icon = "◆" }
end

local function item_for(pane, opts)
	local state = pane.state
	if not priority[state] then
		return nil
	end
	if state == "working" and opts.working == false then
		return nil
	end

	local agent = agent_for(pane)
	if not agent then
		return nil
	end

	local text = (symbols[state] or agent.icon or "◆") .. " " .. (agent.label or agent.name)
	if opts.context ~= false then
		local context = context_for(pane)
		if context ~= "" then
			text = text .. " " .. context
		end
	end
	local color = colors[state] or colors.waiting
	return {
		text = text,
		fg = color.bg or color.fg,
		priority = priority[state],
		pane = pane.id,
	}
end

function tpane.agents.items(opts)
	opts = opts or {}
	local items = {}
	for _, pane in ipairs(tpane.panes()) do
		local item = item_for(pane, opts)
		if item then
			items[#items + 1] = item
		end
	end
	table.sort(items, function(a, b)
		if a.priority ~= b.priority then
			return a.priority < b.priority
		end
		return a.pane < b.pane
	end)
	return items
end

function tpane.agents.update_tabs()
	local windows = {}
	for _, pane in ipairs(tpane.panes()) do
		local item = item_for(pane, { working = true })
		local window = windows[pane.window] or { priority = 999, state = nil, count = 0 }
		if item then
			window.count = window.count + 1
			if item.priority < window.priority then
				window.priority = item.priority
				window.state = pane.state
			end
		end
		windows[pane.window] = window
	end
	for window, item in pairs(windows) do
		tpane.tmux.set_window_var({
			target = window,
			name = "@tpane_agent_indicator",
			value = window_indicator(item.state, item.count),
		})
		tpane.tmux.set_window_var({
			target = window,
			name = "@tpane_agent_tab_style",
			value = window_style(item.state),
		})
	end
end

tpane.on("tick", tpane.agents.update_tabs)
tpane.on("state:change", tpane.agents.update_tabs)

local original_tabline = tpane.tabline
function tpane.tabline(opts)
	opts = opts or {}
	if opts.agent_indicator == false then
		return original_tabline(opts)
	end
	local next_opts = {}
	for key, value in pairs(opts) do
		next_opts[key] = value
	end
	next_opts.prefix = (opts.prefix or "") .. "#{@tpane_agent_tab_style}"
	next_opts.suffix = "#{@tpane_agent_indicator}" .. (opts.suffix or "")
	return original_tabline(next_opts)
end

function tpane.widgets.agents(opts)
	opts = opts or {}
	local max = opts.max or 3
	return tpane.widget(function()
		local parts = {}
		for idx, item in ipairs(tpane.agents.items(opts)) do
			if idx > max then
				break
			end
			if #parts > 0 then
				parts[#parts + 1] = "  "
			end
			parts[#parts + 1] = { text = item.text, fg = item.fg }
		end
		if #parts == 0 then
			return ""
		end
		return parts
	end)
end
