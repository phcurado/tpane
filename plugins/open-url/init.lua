local url_pattern = "[%w][^%s<>'\"%(%[%{%)%]%}>]*[%.:/][^%s<>'\"%(%[%{%)%]%}>]*"

local function sh_quote(value)
	return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function trim(value)
	local out = (value or ""):gsub("^%s+", ""):gsub("%s+$", "")
	return out
end

local function run(command)
	local handle = io.popen(command)
	if not handle then
		return ""
	end
	local output = handle:read("*a") or ""
	handle:close()
	return trim(output)
end

local function tmux(pane, format)
	return run("tmux display-message -p -t " .. sh_quote(pane.id) .. " " .. sh_quote(format))
end

local function opener()
	local configured = os.getenv("TPANE_OPEN_URL")
	if configured and configured ~= "" then
		return configured
	end
	if run("uname") == "Darwin" then
		return "open"
	end
	return "xdg-open"
end

local function open(url)
	os.execute(opener() .. " " .. sh_quote(url) .. " >/dev/null 2>&1 &")
end

local function clean(value)
	value = trim(value)
	value = value:gsub("^[<>'\"%(%[%{]+", "")
	value = value:gsub("[.,;:!?)%]%}>]+$", "")
	return value
end

local function host(value)
	return value:match("^https?://([^/%?#]+)") or value:match("^([^/%?#]+)")
end

local function valid_ipv4(value)
	local parts = { value:match("^(%d+)%.(%d+)%.(%d+)%.(%d+)$") }
	if #parts ~= 4 then
		return false
	end
	for _, part in ipairs(parts) do
		local n = tonumber(part)
		if not n or n > 255 then
			return false
		end
	end
	return true
end

local function valid_host(value)
	value = value and value:gsub(":%d+$", "")
	if value == "localhost" or valid_ipv4(value) then
		return true
	end
	if not value or not value:find("%.") or not value:match("^[%w.-]+$") then
		return false
	end
	local tld = value:match("%.([A-Za-z][A-Za-z0-9-]*)$")
	return tld ~= nil and #tld >= 2
end

local function normalize(value)
	value = clean(value)
	if value == "" then
		return nil
	end
	if value:match("^https?://") then
		return valid_host(host(value)) and value or nil
	end
	if value:match("^localhost[:/]") then
		return "http://" .. value
	end
	if value:match("^www%.") or valid_host(host(value)) then
		return "https://" .. value
	end
end

local function url_at_column(line, column)
	local cursor = column + 1
	local offset = 1
	while true do
		local start, finish = line:find(url_pattern, offset)
		if not start then
			return nil
		end
		if start <= cursor and cursor <= finish then
			return normalize(line:sub(start, finish))
		end
		offset = finish + 1
	end
end

local function visible_line(pane, row)
	local lines = {}
	for line in (pane:capture() .. "\n"):gmatch("(.-)\n") do
		lines[#lines + 1] = line
	end
	return lines[row + 1] or ""
end

local function open_url_under_cursor(pane)
	if tmux(pane, "#{pane_in_mode}") ~= "0" then
		local url = normalize(tmux(pane, "#{copy_cursor_hyperlink}"))
			or url_at_column(tmux(pane, "#{copy_cursor_line}"), tonumber(tmux(pane, "#{copy_cursor_x}")) or 0)
			or normalize(tmux(pane, "#{copy_cursor_word}"))
		if url then
			open(url)
			return
		end
	else
		local column = tonumber(tmux(pane, "#{cursor_x}")) or 0
		local row = tonumber(tmux(pane, "#{cursor_y}")) or 0
		local url = url_at_column(visible_line(pane, row), column)
		if url then
			open(url)
			return
		end
	end

	tpane.tmux.display({ target = pane.id, message = "tpane: no URL under cursor" })
end

tpane.bind("o", open_url_under_cursor, { desc = "Open URL under cursor" })
tpane.bind("C-o", open_url_under_cursor, { mode = "copy", desc = "Open URL under cursor" })
