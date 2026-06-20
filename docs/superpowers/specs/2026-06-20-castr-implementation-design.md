# castr — v1 Implementation Spec

**Date:** 2026-06-20
**Status:** Approved for planning (extensibility-loop-first reset)
**Parent design:** [`../../../2026-06-20-castr-design.md`](../../../2026-06-20-castr-design.md)
**Control UI direction:** [`2026-06-20-castr-control-ui-design.md`](2026-06-20-castr-control-ui-design.md)

This spec pins the decisions left open in the parent design doc and defines the
v1 build. The parent doc is the source of truth for *what* castr is and *why*;
this doc covers *how it ships*.

## Thesis (why castr is worth building)

castr is only worth it if it becomes the **typed, hot-reloadable home for the
user's tmux workflow** — not just pane labels. The product is the *extensibility
loop*: edit a Lua file → hot-reload → instantly see kinds, commands, panels, and
state change. The control pane is a **projection** of registered Lua semantics,
not a hand-designed UI built around tmux hacks.

Concretely, success = the user's existing dotfiles workflow
(`tmux-workspace` toggle/zoom, `@agent_sidebar`/`@bottom_terminal` roles, the
`nvim-approve` approval flow) is **migrated onto a Lua surface the user enjoys
tuning weekly**, replacing fragile AWK/positional parsing and `spawnSync` tmux
gymnastics. Additive-only (castr running beside the scripts forever) is a
failure mode; absorbing the scripts is the win.

## Resolved Decisions

### Daemon lifecycle — `castr` is the front door

`castr` (no verb) replaces `tmux` in the user's flow:

```
$ castr
  ├─ ensure daemon running   (spawn if absent; adopt if already alive)
  ├─ session exists?  → attach   (already attached elsewhere → attach shared/
  │                               read-write gracefully, never a cryptic error)
  └─ no session?      → create + attach
```

- `castr <verb>` is a thin client over a Unix socket. It does **not** spawn the
  daemon — the launcher owns daemon lifecycle.
- **Scope:** one daemon per tmux server; socket path keyed to the tmux server.
- **Stale-socket recovery:** connect failure → treat as stale, remove, respawn.
- **Shutdown:** daemon exits when its tmux server exits. No orphans.

### Lua is the product surface, dogfooded

mlua is embedded from the start. Everything users can do, castr's own built-ins
do **through the same API** — no privileged internal path. The built-in pi/nvim/
term kinds, the built-in agents control panel, and the migrated `ai-toggle`/
`ai-zoom` commands are all ordinary Lua registrations.

### Control pane: declarative data, Rust renders

Lua **declares structured panel data + named action handlers**; Rust owns the
render loop and draws consistently. No `ui.*` immediate-mode drawing API in v1.
A panel returns cards/rows/badges/accents (data) plus `on.{enter,key}` handler
functions castr stores and fires on input. An imperative `render(ui)` form is
deferred until the data model proves too weak — and only then, as a curated
addition. (Matches the control-ui design doc.)

### State: poll by default, push where an extension cooperates

State is detected by a periodic poll of agent panes (~1s) running each kind's
Lua `state` function over `capture-pane` text and readable pane vars. This means
poll-detected state lags up to ~1s.

For signals an extension already emits (the `nvim-approve` approval flow sets
`@pi_approval`), castr also accepts a **push**: `castr set-state <pane> blocked`
over the socket updates state instantly (milliseconds). Push = instant where
cooperation exists; poll = ~1s fallback elsewhere. This is the parent doc's
parked "pi pushes its own state" idea, scoped to the one case that matters.

### Deferred

- `register_keybind` model — parked; keys live in tmux.conf pointing at `castr`
  verbs for now (option A: tmux owns keys, castr owns commands).
- Imperative `render(ui)` UI API — deferred until declarative data is proven.
- macOS `ProcessProvider` backend — after the Linux loop is solid.
- Per-agent capture-pane heuristic tuning — refined as kinds grow.

## Build Plan — extensibility-loop-first

Reordered from the original identity-first slices. Each slice runnable in a real
tmux session before the next. The already-built S1 (launcher + daemon + socket +
mlua + Linux `ProcessProvider` + `register_kind` + pi/nvim/term/claude/copilot
Lua kinds + `@castr_*` border labels) is the **starting point**, not a slice to
redo.

### S2 — Hot-reload loop (the addiction engine)
**Goal:** edit a Lua file, see it take effect, without restarting the daemon.

- Load user plugins from `~/.config/castr/*.lua` (+ subdirs `kinds/`, `panels/`,
  `commands/`) in addition to the built-ins.
- `castr reload` verb + file-watch (or poll) → rebuild the Lua runtime's
  registrations without dropping the daemon or socket.
- **Error isolation:** a Lua file that errors on load is reported (logged +
  surfaced via `castr status`), the previous good registration is kept, and the
  daemon survives. A `state`/`detect`/panel function that throws at runtime is
  caught, the offending unit shows an error, the daemon keeps running.

**Done when:** editing a kind's `label` and saving changes the border label live,
and a deliberately broken plugin file does not crash or wedge the daemon.

### S3 — Lua surface: commands + events + typed primitives
**Goal:** the API needed to express the dotfiles workflow.

- `register_command{ name, handler }` → adds a `castr <name>` verb.
- `on("pane:new"|"pane:focus"|"state:change"|"tick", handler)` event dispatch.
- Small typed tmux primitives exposed to Lua (no raw shell strings):
  `castr.panes()`, `pane:var(name)`, `pane:set{kind=,label=,state=,role=}`,
  `castr.tmux.{split,join,zoom,select,break,stash}` as needed by the migration,
  and `with_pane(pane, {zoom=,state=}, fn)` with guaranteed restore on exit/panic
  (Rust `Drop`).

**Done when:** a Lua command can stage a pane (zoom + set state), run, and
restore even on error, driven by `castr <verb>`.

### S4 — Migrate the real dotfiles
**Goal:** replace the shell glue, keep the muscle memory.

- Reimplement as Lua commands: `ai-toggle`, `ai-zoom`, `term-toggle`,
  `term-zoom`. Keybinds stay `C-a a/A/t/T`, repointed to `run-shell "castr …"`.
- Replace raw role vars with **typed pane roles**: `@agent_sidebar` /
  `@bottom_terminal` become `pane:set{role="agent"|"terminal"}`; hidden-session
  stashing becomes a `stash`/`unstash` primitive. No AWK, no positional
  delimiters.
- `window-unlinked` cleanup → `on("window:close", …)`.

**Done when:** `C-a a/A/t/T` behave as today but run through castr Lua, with no
positional parsing and with correct zoom-safety/restore.

### S5 — Approval state integration
**Goal:** the approval flow lights the control pane, instantly.

- pi kind `state` reads `@pi_approval` (via `pane:var`) → `blocked`; capture-pane
  heuristic fallback for inline prompts.
- `castr set-state <pane> blocked` push verb; `nvim-approve.ts` calls it so the
  signal is instant and the extension sheds its tmux gymnastics over time (the
  diff/edit logic stays; the ~80 lines of `spawnSync("tmux",…)` move to castr).
- seen/unseen model: `working`/`blocked`/`done_unseen`/`idle_seen`;
  `on("pane:focus")` clears `done_unseen → idle_seen`.

**Done when:** triggering a real nvim approval turns the agent pane 🔴 in border
+ control pane, visible even while zoomed elsewhere, with no perceptible lag.

### S6 — Control pane as projection
**Goal:** the dashboard, rendered from registered semantics.

- `register_panel{ id, title, cards = function() … end }` returning declarative
  card data + `on.{enter,key}` handlers (see Resolved Decisions).
- Built-in `agents` + `panes` panels written **through** `register_panel`
  (dogfood). `castr control` opens the overlay TUI; Rust render loop calls each
  panel's `cards()` per frame, lays out, routes input to handlers.
- Reads daemon state over the socket; never scrapes tmux itself; never swallows
  socket errors into empty state.
- Always-on signal: a Lua-formatted compact status surface (kind + state dot per
  agent) for at-a-glance presence across tabs; full interaction lives in the
  overlay. (Status-chrome cannot host an interactive widget tree.)

**Done when:** `castr control` shows agents/panes from live daemon state with
`enter`=jump / `z`=zoom, and a user can add a new panel in Lua that appears with
no Rust change.

### S7 — Cross-platform + polish
- macOS `ps -t <pane_tty> -o pid,ppid,args` `ProcessProvider` backend.
- Heuristic tuning per agent; plugin discovery/load-order docs.

**Done when:** castr runs on macOS with the same kinds/commands/panels.

## Out of Scope for v1

- Migrating beyond the listed scripts; broad `tmux.*` growth past migration
  needs; imperative Lua UI drawing; a generic `tmux.set` escape hatch.

## Architecture Invariants

- Daemon holds authoritative typed state; tmux `@castr_*` vars are a **render
  cache only** — independent simple string fields, never delimited blobs.
- Extensibility = composing a small curated, stable primitive set, dogfooded by
  built-ins. If the API is too weak for a built-in, the API gets fixed.
- Cross-platform via `ProcessProvider`; no `/proc` assumption leaks past the
  Linux backend.
- `with_pane` restores pane state on exit *or panic* via `Drop` — no leaked
  flags.
- A broken plugin must never crash the daemon.
