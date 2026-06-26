local palettes = {}
local aliases = {}

local data = tpane._theme_data
tpane._theme_data = nil

local function normalize(name)
  return tostring(name):lower():gsub("[_-]", " "):gsub("%s+", " "):gsub("^%s+", ""):gsub("%s+$", "")
end

for line in data:gmatch("[^\r\n]+") do
  local name, background, foreground, colors = line:match("^([^\t]+)\t([^\t]+)\t([^\t]+)\t(.+)$")
  if name then
    local palette = {}
    local index = 0
    for color in colors:gmatch("[^,]+") do
      palette[index] = color
      index = index + 1
    end
    palettes[name] = {
      background = background,
      foreground = foreground,
      palette = palette,
    }
    aliases[normalize(name)] = name
  end
end

local function color(palette, index, fallback)
  return palette.palette and palette.palette[index] or fallback
end

local selected
local selected_opts

local function apply(palette, opts)
  opts = opts or {}
  local bg = opts.status_bg or palette.background or palette.bg
  if opts.transparent then bg = "default" end
  local fg = opts.status_fg or palette.foreground or palette.fg
  local red = palette.red or color(palette, 1, fg)
  local green = palette.green or color(palette, 2, fg)
  local yellow = palette.yellow or color(palette, 3, fg)
  local blue = palette.blue or color(palette, 4, fg)
  local muted = palette.muted or color(palette, 8, color(palette, 0, fg))
  local accent = palette.accent or blue or fg

  tpane.opt.status_style = { bg = bg, fg = fg }
  tpane.opt.pane_border_style = { fg = muted }
  tpane.opt.pane_active_border_style = { fg = accent }
  tpane.tabline {
    inactive = { fg = muted },
    current = { fg = accent, bold = true },
  }

  tpane.state("approval", { color = yellow, glyph = "" })
  tpane.state("blocked", { color = red, glyph = "" })
  tpane.state("working", { color = yellow, glyph = "" })
  tpane.state("done_unseen", { color = blue, glyph = "" })
  tpane.state("idle_seen", { color = green, glyph = "" })

  return palette
end

function tpane.theme(theme, opts)
  opts = opts or {}
  if type(theme) == "table" then
    selected = theme
    selected_opts = opts
    return apply(theme, opts)
  end
  local name = aliases[normalize(theme)] or theme
  local palette = palettes[name]
  if not palette then error("unknown theme: " .. tostring(theme), 2) end
  selected = palette
  selected_opts = opts
  return apply(palette, opts)
end

tpane._defer(function()
  if selected then apply(selected, selected_opts) end
end)
