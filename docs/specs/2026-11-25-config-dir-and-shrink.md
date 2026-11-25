# Config folder, split configs, tmux.conf, and a smaller codebase

## Goal

- Config can be split over tidy files and pointed at any folder, the tmux identity reads `~/.tmux.conf` like tmux, and the source tree gets smaller and has no file over 800 lines, with behaviour unchanged.

## Decisions

- Shrink scope: remove dead code, dead files and unused dependencies, and merge duplicate helpers, using the `shrink-anything` CLI. A cut is kept only if `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, the full `cargo test` and `bats tests/cli.bats` stay green. Then split every `src` file over 800 lines. Behaviour is unchanged.
- Add `--config-dir` and `GROFE_CONFIG_DIR`. Every per-folder file comes from that folder. `--config` and `GROFE_CONFIG` still pick the main file.
- Lua `require("keymaps")` and `require("ui.status")` load from the config folder. TOML gets a documented `include = [...]` key, and `source-file` stays as an alias. The docs show a tidy split layout.
- When started as `tmux` with no explicit config, grofe reads the default tmux config files the way tmux does. The grofe identity is unchanged.
- PR order:
  1. Config features (part A).
  2. Shrink cuts, one PR per area.
  3. File splits, one PR per area.
- Ruling (spec review): when started as `tmux` with no explicit config, and at least one default tmux file exists, grofe behaves exactly as if those files had been passed with `-f`. That is today's `-f` path, with no TOML chain and no `settings.toml`. When no default file exists, the TOML chain loads as it does today.
  - Reason: the TOML chain is applied in one go, and `source-file` sets are runtime, so mixing the two would make saves lose on every reload. The `-f` path is already tested and matches tmux.
  - Cost if wrong: a user with both a tmux.conf and TOML files, running grofe as `tmux`, must pass `--config` or `--config-dir` to get the TOML.
- Ruling: a Lua `require` name may not contain `..` and may not be absolute. The path is built lexically under the canonical config folder, and symlinks are followed.
  - Reason: the folder is the user's own, and stow or chezmoi files symlinked into it must work.
  - Cost if wrong: a symlink the user put inside the folder can point outside it.

## Part A: config

### Acceptance criteria

- A1: `grofe --config-dir D` (or `GROFE_CONFIG_DIR=D`) loads these in order:
  - `D/config.toml`
  - `D/conf.d/*.toml`
  - `D/user.toml`
  - `D/settings.toml`
  - `D/init.luau`, or else `D/init.lua`

  `machines.toml`, `pack-lock.toml` and `layouts/` also come from `D`.
- A2: precedence. An empty env var counts as unset.
  - File: `--config`, then `GROFE_CONFIG`, then `<folder>/config.toml`.
  - Folder: `--config-dir`, then `GROFE_CONFIG_DIR`, then the parent of the file named by `--config` or `GROFE_CONFIG`, then XDG.
  - So `--config F --config-dir D` reads `F` plus every other layer from `D`.
  - Only `--config F` keeps today's behaviour: the folder is `F`'s parent.
- A3: the server started by the client gets the same folder.
  - The folder is passed explicitly, also through `__config-file`, `server_config_dir` and the layout client path.
  - `reload`, the settings menu Apply and customize `P` all read and write `D/settings.toml`.
  - A client whose folder differs from the running server's gets the same warning as one whose file differs.
- A4: a missing folder is created on the first write, as with `--config` today. A folder path that is a file is an error that names the path.
- A5: `grofe --help` and the book list `--config-dir` and `GROFE_CONFIG_DIR`.
- A6: `require("keymaps")` loads the first of these that exists:
  - `D/keymaps.luau`
  - `D/keymaps.lua`
  - `D/keymaps/init.luau`
  - `D/keymaps/init.lua`

  `require("ui.status")` loads `D/ui/status.luau`, or else `D/ui/status.lua`. A leading `./` is allowed. The call returns the module's value.
- A7: a module required twice in one load runs once and returns the same value. Each reload starts with an empty cache.
- A8: each of these is a Lua error:
  - a name with `..`
  - an absolute path
  - an empty name
  - a name with a file extension
  - a missing module: the error names the module and the paths tried
  - a require cycle

  The 50 ms budget and the memory limit cover required modules. Symlinks are followed, per the ruling above.
- A9: `require` is a Rust function that replaces mlua's global `require`.
  - It is installed before `sandbox(true)`.
  - It reads the folder from a registry slot that the init load sets, so it also works in callbacks such as `lua-call` after init returns.
  - Plugin and pack code keep their current behaviour, as the existing tests check.
- A10: TOML `include = "keymaps.toml"` or `include = ["keymaps.toml", "theme.toml"]` behaves exactly like `source-file`:
  - paths are relative to the including file
  - cycles are detected
  - a missing file is an error
  - included files apply after the including file's own keys

  When both keys are given, `include` applies first, then `source-file`. Errors name the key that was actually used.
- A11: when started as `tmux` (argv0, `started_as_tmux()`), with none of `-f`, `--config` or `--config-dir`:
  - The candidates, in order, are:
    - `/etc/tmux.conf`
    - `~/.tmux.conf`
    - `$XDG_CONFIG_HOME/tmux/tmux.conf`, only when that variable is set and not empty
    - `~/.config/tmux/tmux.conf`
  - A candidate whose expanded path string equals an earlier one is dropped.
  - If at least one candidate exists, grofe behaves exactly as `-f <each existing candidate>` does today: load order, parse-error reporting, `reload` and `config_files`.
  - `~` is `$HOME`, or the passwd home when `HOME` is unset.
  - The order was checked against real tmux 3.8 in the spec review: every existing file loads, in this order. The test uses a fixture, not a live tmux.
- A12: under the grofe identity, no tmux file is read without `-f`. An explicit `-f`, `--config` or `--config-dir` wins over the tmux defaults. With no default tmux file present, today's TOML chain loads.
- A13: the book's configuration page documents:
  - the folder option and its precedence
  - `include`
  - Lua `require`
  - tmux.conf loading
  - an example tidy layout: `config.toml` with `include = ["keymaps.toml", "theme.toml"]`, plus `init.luau` doing `require("keymaps")` and `require("ui.status")`, with `keymaps.luau` and `ui/status.luau`
- A14: the book's Lua page describes `require` exactly as built, and drops the wrong claim it makes today.

### Edge cases

- A folder path with spaces or unicode (`Kønfig dir`).
- An empty `conf.d`.
- `GROFE_CONFIG_DIR` set but empty.
- A `--config-dir` that is a symlink to a folder.
- `require("")`, `require("a..b")`, `require("keymaps.luau")`, and a module that errors when it loads.
- `include = []`, `include = 3` (a type error that names the key), and an include that points at itself.
- tmux identity:
  - When `~/.tmux.conf` and `~/.config/tmux/tmux.conf` are the same file through a symlink, it loads twice, as in tmux.
  - With `XDG_CONFIG_HOME=$HOME/.config`, that file loads once.
  - When `HOME` is unset, the passwd home is used.

### Test list

- t1 (e2e) A1: `config.toml` has `[status]` with `left = "A"`, and `conf.d/x.toml` has `[status]` with `right = "B"`. Both values apply under the flag and under the env var.
- t2 (e2e) A2: `--config F --config-dir D` takes the option from `F` and `user.toml` from `D`. Under the env-var variants, an empty env var is ignored.
- t3 (e2e) A3: a settings menu Apply writes `D/settings.toml`, and a second client with a different folder gets the warning.
- t4 (unit) A4: a missing folder is created on write, and a path that is a file gives an error.
- t5 (bats) A5: the help text.
- t6 (e2e) A6: `init.luau` calls `require("keymaps")`, which binds a key, and `require("ui.status")`, which sets an option.
- t7 (unit) A7: a counter module required twice leaves the counter at 1 and returns the same table.
- t8 (unit) A8 and A9:
  - Each of these is an error: `..`, `/etc/passwd`, `""`, `keymaps.luau`, a missing module (the error lists the paths tried), a cycle, and a module that runs past the budget.
  - A symlinked module file that lives outside the folder loads.
  - `require` called from a `lua-call` callback after init works.
- t9 (unit) A10: a string, a list, relative paths, a cycle, a missing file, `include = 3`, `include = []`, both keys (order), and the key name in errors.
- t10 (e2e) A11, with `HOME` a temp folder and `XDG_CONFIG_HOME` unset:
  - A `~/.tmux.conf` with `set -g status-left X` gives `X`.
  - `~/.tmux.conf` (`set -g @a 1` and `set -g @order a`) and `~/.config/tmux/tmux.conf` (`set -g @order b`) together give `@order b` and `@a 1`.
  - With `XDG_CONFIG_HOME=$HOME/.config`, a `set -ga @n x` in that file gives `x`.
  - A grofe `config.toml` is ignored when a tmux file exists, and used when none does.
- t11 (e2e) A12: the grofe identity with the same `HOME` ignores `~/.tmux.conf`, and under the tmux identity `--config-dir` wins over `~/.tmux.conf`.
- t12: the docs and options tests pass, and `just docs` is clean.
## Part B: shrink

### Acceptance criteria

- B1: net line count of `src` and `Cargo.toml` dependencies goes down, reported per PR (`shrink-anything report`).
- B2: no CLI flag, command, option, format variable, hook, key binding or TOML key is removed. The options reference and `grofe --help` are unchanged byte for byte, except for Part A's additions.
- B3: no test is deleted or weakened. Tests are code too, but they are only merged when they duplicate each other exactly.
- B4: before cutting, list the code no test reaches (the `cargo llvm-cov` uncovered list, or the tool's untested-area warning) per area. Cuts in such code need a reason in the PR body: a grep showing no caller, macro, `cfg` or reflective path reaches it.
- B5: every PR passes CI on Linux and macOS, cuts in `cfg(target_os)` code need the Linux leg green, and `just gate canopy athome` runs once before the release.

## Part C: splits

### Acceptance criteria

- C1: no file under `src/` is over 800 lines, test modules included.
- C2: splits move code by responsibility into submodules. Moves change only visibility and `use` lines, with no logic changes, so `git diff --color-moved` shows moved blocks.
- C3: public paths used by other crates or tests stay valid (re-export when needed).
- C4: each PR passes CI, and part B's B2 and B3 hold.

## Out of scope

- New features beyond part A.
- Renaming options or commands.
- Changing tmux-compat behaviour, except A11.
- Performance work.
