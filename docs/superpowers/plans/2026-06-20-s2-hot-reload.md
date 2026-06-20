# Plan — S2: Hot-reload loop

**Spec:** `../specs/2026-06-20-castr-implementation-design.md` (slice S2)
**Goal:** edit a Lua file under the config dir, see it take effect without
restarting the daemon; a broken plugin never crashes or wedges the daemon, and
its error is retrievable.

## Current state (starting point)

- `src/lua_runtime.rs` — `LuaRuntime { lua, kinds }`. `new()` installs the API
  then loads user kinds (`config_dir()/kinds/*.lua`) then builtins. `detect()`
  iterates kinds, returns first match. `register_kind` reads `name/detect/label`.
  `config_dir()` resolves `CASTR_CONFIG_DIR` → `XDG_CONFIG_HOME/castr` →
  `HOME/.config/castr`.
- `src/daemon.rs` — `Daemon { lua, process_provider, panes }`. `run()` loop:
  accept socket, `scan()` every 1s, exit when tmux server dies. `handle()`
  dispatches requests.
- `src/protocol.rs` — `Request { Ping, Refresh, Pick, Panes, SelectPane }`,
  `Response { ok, data, error }`.
- `src/main.rs` — subcommands `Daemon`, `Refresh`, `Ping`, `Pick`; `request_at`
  socket client helpers.

## Design decisions (already settled — implement these, don't re-litigate)

1. **Fresh-runtime-swap.** Reload builds a brand-new `LuaRuntime` and only
   replaces the live one if the builtins load. The old runtime keeps serving if
   the rebuild fails catastrophically. (mlua `Lua` is single-threaded; the daemon
   loop is single-threaded; a plain field swap is fine.)
2. **Per-file last-good fallback.** The daemon caches the last source string that
   loaded cleanly per file path. On reload, a file that now errors falls back to
   its cached good source, and the error is recorded. Cache lives in the daemon
   (survives runtime swaps), not in `LuaRuntime`.
3. **Trigger = mtime poll + explicit verb.** The daemon loop checks config-file
   mtimes each tick and reloads on change. `castr reload` forces it. No new
   crate dependency — poll, don't use a filesystem-watch crate.
4. **Load order:** user plugins first, builtins last (keeps the `term` catch-all
   as the final fallback; user kinds win).
5. **Resilient `detect`.** A kind whose `detect`/`label` throws is skipped, not
   fatal; the scan continues with the remaining kinds.
6. **Errors are retrievable.** `castr status` returns current load errors (the
   daemon is spawned with stdout/stderr to `/dev/null`, so `eprintln!` is not a
   reporting channel).

## Tasks

### 1. `lua_runtime.rs` — split creation from loading
- `new()` installs the API only (no kind loading).
- Add `load_builtins(&self) -> Result<()>` running the existing `BUILTIN_KINDS`.
- Add `load_source(&self, name: &str, source: &str) -> Result<()>` executing one
  chunk (replaces the inline load in `load_user_kind`).
- Add `kind_count(&self) -> usize`.
- Add `pub fn user_plugin_files() -> Vec<PathBuf>`: sorted `*.lua` under
  `config_dir()` root and subdirs `kinds/`, `panels/`, `commands/`. Missing dirs
  yield nothing. Deterministic order. Make `config_dir()` `pub(crate)`.
- Remove `load_user_kinds` / `load_user_kind` / `load_builtin_kinds` (logic moves
  to the daemon + the two new methods).
- `detect()`: wrap each kind's `detect`/`label` call so an `Err` skips that kind
  and continues instead of returning `Err`. Keep infra errors (userdata
  creation) as `Err`.

### 2. `daemon.rs` — own the reload + cache
- Extend `Daemon` with: `last_good: HashMap<PathBuf, String>`,
  `load_errors: Vec<String>`, `config_sig: Vec<(PathBuf, SystemTime)>`.
- Add `reload_plugins(&mut self)`:
  1. `let rt = LuaRuntime::new()?; rt.load_builtins()?;` (builtins must succeed;
     if not, keep `self.lua`, record error, return).
  2. For each `user_plugin_files()` path (in order): read content; try
     `rt.load_source(name, &content)`. On `Ok`, update `last_good`. On `Err`,
     push the error to a fresh error list and, if `last_good` has a prior good
     source for that path, load that instead.
  3. Swap: `self.lua = rt; self.load_errors = errors;`.
  Note builtins load *after* the runtime is constructed but the load order
  requirement (user first) is about kind precedence — load user files first, then
  call `rt.load_builtins()` last. Adjust ordering accordingly.
- Add `config_changed(&self) -> Option<Vec<(PathBuf, SystemTime)>>`: compute the
  current `(path, mtime)` signature from `user_plugin_files()`; return `Some(new)`
  if it differs from `self.config_sig`, else `None`.
- `run()` loop: each tick, if `config_changed()` is `Some(sig)`, call
  `reload_plugins()` and store `sig`. Keep the existing 1s scan + server-alive
  exit.
- `Daemon::new()` calls `reload_plugins()` for the initial load and seeds
  `config_sig`.
- `handle()`: add `Request::Reload =>` run `reload_plugins()`, return ok with
  `"reloaded N kinds, M errors"`. Add `Request::Status =>` return `load_errors`
  joined (or `"ok"` when empty).

### 3. `protocol.rs`
- Add `Reload` and `Status` variants to `Request`.

### 4. `main.rs`
- Add `Reload` and `Status` subcommands mapping to `Request::Reload` /
  `Request::Status`, printed via the existing `print_response`.

## Done when

- Editing a kind's `label` in `~/.config/castr/kinds/*.lua` and saving changes
  the tmux border label within ~1 poll tick, no daemon restart.
- `castr reload` forces an immediate reload.
- A `*.lua` file with a syntax error: daemon keeps running, the previous good
  version of that file stays active, and `castr status` reports the error.
- A kind whose `detect` throws at runtime does not abort the scan for other
  panes/kinds.

## Out of scope (later slices)

- `register_command`, `on()`, typed tmux primitives, `with_pane` (S3).
- Dotfiles migration (S4), approval state (S5), control pane (S6).
- Surfacing *runtime* (non-load) errors via `status` — load errors only for S2.
