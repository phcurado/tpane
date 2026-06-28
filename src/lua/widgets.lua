tpane.widgets.session = tpane.widget(function()
  return "[#{client_session}]"
end)

tpane.widgets.host = tpane.widget(function()
  return "#H"
end)

tpane.widgets.clock = tpane.widget(function()
  return os.date("%H:%M")
end)

tpane.widgets.date = tpane.widget(function()
  return os.date("%b %d")
end)

tpane.widgets.prefix = tpane.widget(function()
  return tpane.fmt.prefix("  ", "  ")
end)

tpane.widgets.tabs = tpane.widget(function()
  return "#{W:#{E:window-status-format} ,#{E:window-status-current-format} }"
end)

local battery_cmd = [[
if [ "$(uname)" = "Darwin" ]; then
  pmset -g batt 2>/dev/null | awk '
    /Battery/ {
      match($0, /[0-9]+%/)
      pct = substr($0, RSTART, RLENGTH)
      if (pct == "") next
      icon = "󰁹"
      if ($0 ~ /charging/) icon = "󰂄"
      if ($0 ~ /discharging/) icon = "󰁹"
      print icon " " pct
      exit
    }
  '
else
  for bat in /sys/class/power_supply/BAT*; do
    [ -d "$bat" ] || continue
    cap=$(cat "$bat/capacity" 2>/dev/null)
    status=$(cat "$bat/status" 2>/dev/null)
    [ -n "$cap" ] || continue
    icon="󰁹"
    if [ "$status" = "Charging" ]; then
      icon="󰂄"
    elif [ "$status" = "Discharging" ]; then
      if [ "$cap" -le 15 ]; then icon="󰁺";
      elif [ "$cap" -le 30 ]; then icon="󰁻";
      elif [ "$cap" -le 50 ]; then icon="󰁽";
      elif [ "$cap" -le 80 ]; then icon="󰁿";
      else icon="󰁹"; fi
    fi
    printf "%s %s%%\n" "$icon" "$cap"
    exit 0
  done
fi
]]

function tpane.widgets.battery(opts)
  opts = opts or {}
  return tpane.job({
    every = opts.every or "30s",
    timeout = opts.timeout or "3s",
    cmd = opts.cmd or battery_cmd,
  })
end

local player_cmd = [[
if command -v playerctl >/dev/null 2>&1; then
  if [ "$(playerctl status 2>/dev/null)" = "Playing" ]; then
    playerctl metadata --format ' {{artist}} — {{title}}' 2>/dev/null
  fi
elif command -v osascript >/dev/null 2>&1; then
  osascript 2>/dev/null <<'APPLESCRIPT'
tell application "System Events"
  set spotifyRunning to exists process "Spotify"
  set musicRunning to exists process "Music"
end tell

if spotifyRunning then
  tell application "Spotify"
    if player state is playing then return " " & artist of current track & " — " & name of current track
  end tell
end if

if musicRunning then
  tell application "Music"
    if player state is playing then return " " & artist of current track & " — " & name of current track
  end tell
end if
APPLESCRIPT
fi
]]

function tpane.widgets.player(opts)
  opts = opts or {}
  return tpane.job({
    every = opts.every or "5s",
    timeout = opts.timeout or "3s",
    cmd = opts.cmd or player_cmd,
  })
end

local cpu_cmd = [[
if [ "$(uname)" = "Darwin" ]; then
  top -l 1 -n 0 2>/dev/null | awk -F'[:,%]' '/CPU usage/ {
    user = $2 + 0
    sys = $4 + 0
    printf " %.0f%%\n", user + sys
    exit
  }'
elif [ -r /proc/stat ]; then
  read cpu user nice system idle iowait irq softirq steal guest guest_nice < /proc/stat
  idle1=$((idle + iowait))
  total1=$((user + nice + system + idle + iowait + irq + softirq + steal))
  sleep 0.2
  read cpu user nice system idle iowait irq softirq steal guest guest_nice < /proc/stat
  idle2=$((idle + iowait))
  total2=$((user + nice + system + idle + iowait + irq + softirq + steal))
  total=$((total2 - total1))
  idle=$((idle2 - idle1))
  if [ "$total" -gt 0 ]; then
    awk -v total="$total" -v idle="$idle" 'BEGIN { printf " %.0f%%\n", 100 * (total - idle) / total }'
  fi
fi
]]

function tpane.widgets.cpu(opts)
  opts = opts or {}
  return tpane.job({
    every = opts.every or "2s",
    timeout = opts.timeout or "3s",
    cmd = opts.cmd or cpu_cmd,
  })
end

local memory_cmd = [[
if [ "$(uname)" = "Darwin" ]; then
  pagesize=$(pagesize 2>/dev/null || echo 4096)
  total=$(sysctl -n hw.memsize 2>/dev/null)
  free=$(vm_stat 2>/dev/null | awk '/Pages free/ { gsub(/\./, "", $3); print $3 }')
  if [ -n "$total" ] && [ -n "$free" ]; then
    used=$((total - free * pagesize))
    awk -v used="$used" 'BEGIN { printf " %.1fG\n", used / 1024 / 1024 / 1024 }'
  fi
elif [ -r /proc/meminfo ]; then
  awk '
    /MemTotal:/ { total = $2 }
    /MemAvailable:/ { available = $2 }
    END {
      if (total > 0 && available > 0) {
        used = (total - available) / 1024 / 1024
        printf " %.1fG\n", used
      }
    }
  ' /proc/meminfo
fi
]]

function tpane.widgets.memory(opts)
  opts = opts or {}
  return tpane.job({
    every = opts.every or "5s",
    timeout = opts.timeout or "3s",
    cmd = opts.cmd or memory_cmd,
  })
end

setmetatable(tpane.widgets, {
  __index = function(_, name)
    error("unknown widget: tpane.widgets." .. tostring(name), 2)
  end,
})
