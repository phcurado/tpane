# castr — Lua Kind Plugins

**Status:** Implemented initial loading

castr loads user kind plugins from:

```text
~/.config/castr/kinds/*.lua
```

or, if set:

```text
$CASTR_CONFIG_DIR/kinds/*.lua
```

Files load in lexicographic order before built-in kinds. This lets user kinds match
before the built-in `term` fallback.

Example:

```lua
castr.register_kind {
  name = "db",
  detect = function(p)
    return p:proc_tree():any(function(proc)
      return proc.argv:match("psql") ~= nil
    end)
  end,
  label = function(p)
    return "db · " .. p.cwd_basename
  end,
}
```

A broken plugin is skipped and logged to stderr; the daemon continues loading other
plugins and built-ins.

Run this after editing plugins:

```bash
castr refresh
```

`refresh` reloads Lua kind files and rescans panes.
