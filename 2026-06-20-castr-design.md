# castr — Identity-Aware Layer for tmux

**Date:** 2026-06-20
**Status:** Draft, pending design review

## Problem

The current dotfiles setup glues tmux + neovim + pi (AI coding agent) together with
shell scripts and pane metadata. Two real pains:

1. **No pane identity.** tmux panes are nameless. When a pane is zoomed fullscreen
   (`Ctrl-A`/`Ctrl-T` bindings), there is no label or indicator of *what* is running
   there or *what* is hidden behind it. The user loses orientation.
2. **Fragile glue.** Pane orchestration lives in `~/.local/bin/tmux-workspace`
   (POSIX shell, AWK-parses pane metadata positionally) and inline `spawnSync("tmux", …)`
   juggling inside `.pi/agent/extensions/nvim-approve.ts`. Positional parsing breaks on
   delimiters; no validation that panes exist before acting; state smeared across env
   vars, `@pi_approval` flags, and hidden sessions.

herdr (a Rust *terminal multiplexer* built for AI agents) solves identity by replacing
tmux. We do not want to replace tmux — we want tmux's identity layer to exist.

## Goal

A tool that gives tmux the missing identity layer:

- Every pane knows its **kind** (pi / nvim / term / claude / …), a human **label**, and
  (for agents) a **state** (working / idle / blocked / done).
- The user never loses orientation, especially when zoomed.
- A first-class **control pane** keeps the herd visible: workspaces, agents, panes,
  and state rollups in a Herdr-inspired sidebar without replacing tmux.
- All behavior is **Lua-extensible** — unlimited *combinations* of a small, stable
  primitive set. Any user can add a pane-kind or a command without recompiling.
- Works on **Linux and macOS** from the first iteration.

tmux remains the renderer and persistence engine. castr is the brain that decides
*what* and *when*.

## Non-Goals

- **Not a multiplexer.** No terminal emulation, no pane persistence, no copy-mode, no
  detach/reattach. tmux owns all of that.
- **No agent cooperation required.** State detection is heuristic (like herdr), uniform
  across all agents. (A future optional "pi pushes its own state" enhancement is parked —
  not in scope. See Parked Ideas.)
- **No raw expose-everything API.** Extensibility comes from composing a curated stable
  primitive set, not from exposing core internals.

## Architecture

```
┌─ castr daemon (Rust) ──────────────────────────────┐
│  hooks + low-freq poll → kind  (ProcessProvider)     │
│  poll (agent panes)    → state (capture-pane)        │  daemon owns typed
│  Lua runtime (mlua) hosts plugins                    │  state; writes render
│  ── source of truth: in-memory typed state ──        │  cache to tmux vars
└──────────────────────────────────────────────────────┘
        ↑ built-in kinds (pi, nvim, term) are themselves Lua plugins
castr <verb>  (CLI/control TUI) ──Unix socket──▶ daemon
tmux  ──reads @castr_* vars──▶ renders pane-border + managed control pane
```

The daemon holds the authoritative typed state. The tmux pane variables
(`@castr_kind`, `@castr_label`, `@castr_state`, `@castr_summary`) are a **render
cache only** — independent simple string fields the daemon writes so tmux can draw
borders/status without querying the daemon. No serialized/delimited blobs.

- **castr daemon** — long-running background process. Runs detection, hosts the Lua
  runtime, owns the Unix socket. Started on first tmux session (via tmux config) or on
  demand.
- **castr CLI** — front door and thin client. `castr sidebar`, `castr control`, and
  future plugin verbs send requests to the daemon over the Unix socket.
- **Lua runtime (mlua)** — loads plugins from `~/.config/castr/`. castr-core's own
  built-in kinds load through the *same* API exposed to users (dogfood rule — no
  privileged internal path).
- **ProcessProvider** — platform abstraction for inspecting a pane's process tree.
  Linux backend reads `/proc`; macOS backend shells `ps -t <pane_tty> -o pid,ppid,args`.
  No `/proc` assumption leaks into the rest of the architecture.
- **tmux** — renders pane borders from variables; persists sessions; hosts a managed
  castr control pane; does the actual splitting/zooming when castr asks.

### Why a daemon

Two reasons a resident process is required, not a per-invocation CLI:

1. **Kind detection can't be hook-only.** tmux fires hooks on pane create/focus, but the
   *foreground command can change inside an existing pane with no hook* — e.g. typing
   `pi` at a live shell prompt. So kind detection runs on hooks **plus a low-frequency
   poll** of all non-sidebar panes to catch in-pane command changes.
2. **State detection needs live polling.** Heuristic state (working/idle/blocked dots)
   needs a periodic `capture-pane`; without a resident process dots only update on
   focus/keypress. State polling runs **only on agent-tagged panes** (cheap).

## Primitive API (the stable extension contract)

The whole surface. Plugins compose these; the set stays small and stable so plugins
survive version bumps.

**Query**
- `panes()` → list of `{id, pid, window, geometry, zoomed, kind, label, state, cwd}`
- `pane:proc_tree()` → processes under the pane (input for kind detection)
- `pane:capture()` → visible text of the pane (input for state heuristics)

**Mutate** (typed, validated — each checks the pane exists; no raw string building)
- `pane:set{kind=, label=, state=}`
- `tmux.split / tmux.join / tmux.zoom / tmux.select` (specific verbs only)
- *No generic `tmux.set` in the stable surface* — it would be a raw escape hatch that
  contradicts the curated-API rule. A scoped escape hatch can be added later if a real
  plugin needs it, named as such.

**Compose**
- `with_pane(pane, {zoom=, state=}, cmd)` → stage a pane (capture current zoom/active,
  select+zoom, set state), run `cmd` to completion, then restore on exit *or panic*
  (Rust `Drop` guarantees cleanup — no leaked flags).
- `control_view(panes, opts)` → Herdr-inspired control surface data model; the Rust TUI
  renders it first, with Lua hooks added only when real formatting needs appear.
- `pick(panes, opts)` → optional transient switcher built from the same control-view
  model; secondary to the persistent control pane.

**React** (the events analog of pi.dev's `.on()`)
- `on("pane:new" | "pane:focus" | "state:change" | "tick", handler)`

**Register**
- `register_kind{name, detect, label, state, color}`
- `register_command(name, handler)` → adds a `castr <name>` verb
- `register_keybind` — **parked.** tmux keybindings are config mutation
  (`tmux bind-key …`), not runtime-internal like pi extensions. A clean model is needed
  first: does castr install binds, only print snippets, reconcile on hot-reload, handle
  user-owned conflicts? Deferred to its own design pass.

## Detection & Rendering Pipeline

1. **Kind** — on tmux hooks (`after-split-window`, `pane-focus-in`) **plus a low-frequency
   poll of all non-sidebar panes**, because the foreground command can change inside an
   existing pane without any tmux hook firing (e.g. typing `pi` at a shell prompt). Via
   `ProcessProvider`, walk the pane's process tree from `pane_pid`, match each process
   against registered `detect` predicates → set `@castr_kind` and `@castr_label`.
2. **State** — daemon polls agent-tagged panes (~1–2s): `capture-pane -p`, run the kind's
   `state` heuristic (regex over visible output + process activity) → set `@castr_state`.
3. **Render** — tmux and the control TUI read daemon state/render-cache vars:
   - `pane-border-status` shows `@castr_label` per pane
   - zoom/fullscreen orientation is handled by a future visual design, not status-line
     prose or verbose border text
   - `castr control` will render a Herdr-inspired full-screen/overlay control surface
     after a dedicated UI design pass
   - the control pane lists every pane by label/kind/state, including hidden panes
   - state drives a color dot (working 🟡 / idle 🟢 / blocked 🔴 / done 🔵)

### State model (seen/unseen)

Detection alone can tell *working* from *blocked* from *finished*, but not whether the
**user has seen** a finished result — so `done` and `idle` are ambiguous without tracking
attention. The model:

```
working      — agent actively producing
blocked      — agent waiting on the user (approval / y-n prompt)
done_unseen  — agent finished, user hasn't focused the pane since
idle_seen    — finished and the user has looked (focus event fired)
```

A `pane:focus` event transitions `done_unseen → idle_seen`. The done 🔵 dot means
"finished, needs your eyes"; it clears to idle 🟢 once you focus the pane.

### A pane-kind is a plugin (built-in and user, identical)

```lua
castr.register_kind {
  name   = "pi",
  detect = function(p)
    return p:proc_tree():any(function(x) return x.argv:match("pi%-coding%-agent") end)
  end,
  label  = function(p) return "pi · " .. p.cwd_basename end,
  state  = function(p)
    local out = p:capture()
    if out:match("%(y/n%)")             then return "blocked" end
    if out:match("[esc] to interrupt")  then return "working" end
    return "idle"
  end,
  color = "yellow",
}
```

A user adding a `db` kind for psql panes writes the same shape into
`~/.config/castr/kinds/db.lua`, hot-reloads, done. No recompile, no fork.

## Scope

### v1 — Control-pane-first identity layer

- castr daemon (Rust) + Unix socket + Lua runtime
- `ProcessProvider` with **both** Linux (`/proc`) and macOS (`ps`) backends
- `register_kind` API (the most valuable extension point)
- Built-in kinds (as Lua plugins): pi, nvim, term, plus claude/copilot heuristic detectors
- Rendering: pane-border labels and, after a design pass, a Herdr-inspired
  `castr control` full-screen/overlay UI
- seen/unseen state model
- `castr pick` may be added as a secondary transient view, but the persistent control
  pane is the primary v1 surface

**v1 explicitly leaves `~/.local/bin/tmux-workspace` and
`.pi/agent/extensions/nvim-approve.ts` untouched.** It adds the identity layer alongside
the existing setup; nothing is migrated yet.

### Later phases

- **P2** — `register_command` + `with_pane` primitives; migrate the fragile glue into
  castr verbs:
  - `tmux-workspace` toggle/zoom logic → `castr` commands
  - the tmux juggling inside `nvim-approve.ts` → a single
    `castr with-pane <pane> --zoom --state blocked -- nvim -d …` call. The diff UI / edit
    logic stays in the pi extension; only the ~80 lines of `spawnSync("tmux",…)` move.
  - Synergy: `--state blocked` means an approval request auto-lights the pane red 🔴 in
    the border + `pick`, visible even when zoomed elsewhere.
- **P3** — grow the primitive surface (more events, more `tmux.*`) as real plugins demand
  it. Same dogfood and stability rules.

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Heuristic state misfires | Per-kind patterns live in Lua, user-tunable; not hardcoded |
| Daemon poll cost | Poll only agent-tagged panes; kind detection on hooks, not a timer |
| Lua API churn breaks plugins | Small curated surface, dogfooded by built-in kinds — if it's too weak for core, it gets fixed before release |
| Pane var parsing fragility (today's bug) | Daemon owns typed state; tmux vars are independent simple fields (`@castr_kind`/`@castr_label`/`@castr_state`), each read whole — no delimiters, no positional AWK |
| Cross-platform process inspection | `ProcessProvider` trait with Linux (`/proc`) and macOS (`ps`) backends, both shipped in v1 |
| `done` vs `idle` ambiguity | Explicit seen/unseen state model; `pane:focus` clears `done_unseen` |

## Parked Ideas (not in scope)

- **Pi pushes its own state.** Because the user controls `.pi/agent/extensions/`, a pi
  extension could emit exact lifecycle state (`turn:start`/`idle`/`tool:approval-wait`)
  into a tmux var — more accurate than scraping. Parked: heuristics match herdr and keep
  detection uniform/decoupled. Revisit only if pi heuristics prove flaky.

## Resolved Decisions

- **Language:** Rust core, Lua plugins (mlua).
- **Platform:** cross-platform from v1 — Linux + macOS, via `ProcessProvider`.
- **v1 scope:** full-featured (daemon, Lua, built-in kinds, labels, managed control
  pane, state dots). User accepts a larger first build rather than hard phasing.
- **Detection:** heuristic-only (pi-push parked); kind = hooks + low-freq poll; state =
  poll agent panes.
- **State:** typed in the daemon; tmux vars are render cache; seen/unseen model.
- **Zoom orientation:** visual design deferred; no status-line prose or verbose border text.

## Open Questions (to resolve during implementation planning)

- **Daemon lifecycle** — how it starts (tmux config hook vs CLI auto-spawn), one daemon
  per host vs per tmux server, shutdown/restart, stale-socket recovery.
- **Multiple tmux servers/sessions** — socket scoping and pane-id namespacing when more
  than one tmux server is running.
- **Control pane details** — exact sidebar lifecycle, default width, toggle behavior,
  and how much of the Herdr-inspired view is fixed Rust UI vs Lua-customizable data.
- **Lua plugin loading** — discovery paths, load order, hot-reload semantics, error
  isolation (a broken plugin must not crash the daemon).
- **`register_keybind` model** — deferred; needs its own design pass (see Primitive API).
- **Heuristic tuning** — per-agent capture-pane patterns; how users override built-ins.
