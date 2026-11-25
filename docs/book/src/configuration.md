# Configuration

## Where config lives

Configuration is TOML in a config folder: `~/.config/grofe/` by default, or
`$XDG_CONFIG_HOME/grofe/` when that variable is set. Files load in this
order, each overriding the ones before:

1. `config.toml`
2. every `conf.d/*.toml`, in name order
3. `user.toml`
4. `settings.toml`

Then `init.luau` (or else `init.lua`) runs, after every pack (see
[Lua scripting and packs](lua.md)). The folder also holds `layouts/`, the
pack lock `pack-lock.toml` and `machines.toml`. A missing file is skipped, and
a missing folder loads the built-in defaults.

`user.toml` is your own file, so tools and themes can add to your setup
without overriding you. `settings.toml` is the one grofe writes: it loads
last and overrides everything above it, so a value saved there always wins.
Do not expect to edit it by hand; grofe re-reads it before each save and
keeps your comments and other keys, but its layout is its own. Resetting an
option or a key binding removes its entry, and the value or binding from
the layer below applies again.

## Saving from customize-mode

In customize-mode (`prefix C` in the tmux identity), `P` writes the selected
option or key binding to `settings.toml` and reloads the config. `S` is not
used for this: as in tmux, it sets the option globally. A key binding is
saved with its command, its note (`desc`), its repeat flag, its `group` and
its `name`; saving it again replaces the whole entry, so a field the binding
no longer has (a note it was rebound without) leaves the file. Once saved, `settings.toml` owns it: a hand edit of the file and a
`reload` apply, even for a value or binding first set at runtime.

Only global values are saveable: a value set on one session, window or pane
answers `only global values are saveable`. A binding that is not grofe-dialect (a stock key of the
tmux identity, or a `bind-key` from a tmux client) answers `only grofe-dialect
bindings are saveable`. A key bound by a plugin (`init.luau` or a pack) answers
`bound by a plugin, not saveable`, and `d`/`D` leave `settings.toml` alone for it. An option with no TOML key (a
`@user` option or a list option) answers `not saveable`, and its preview in
customize-mode says so. If the reload fails, the file is put back as it was
and the error is shown. A missing `settings.toml` or config folder is created
on the first save.

`d` and `D` (reset) on a global option or a key binding take its entry (an
option's key, a binding's `[keys.<table>]` entry) out of `settings.toml` and
reload: the value or binding from the layer below, such as `user.toml`, or
else the built-in default, applies again, and the config owns it from then on.
If the reload fails, the file, the value and the binding stay as they were
before the reset and the error is shown. On a value set on one session,
window or pane, `d` unsets it as tmux does and leaves `settings.toml` alone.

A server started with `tmux -f <file>` reads only the files you name, never
`settings.toml`, so `P` there answers that nothing was saved.

## Another config folder

`grofe --config-dir <dir>` reads the whole config from `<dir>` in place of
`~/.config/grofe/`, and `GROFE_CONFIG_DIR=<dir>` does the same. Everything
in the list above follows the folder: `config.toml`, `conf.d/`, `user.toml`,
`settings.toml`, `init.luau`, `machines.toml`, `pack-lock.toml` and
`layouts/`. Saves from customize-mode and the settings menu go to
`<dir>/settings.toml`.

A relative path is taken from the current directory and `~/` from `$HOME`.
Spaces and other characters in the path are fine. A folder that does not
exist yet loads as empty, and the first save creates it. A path that is a
file is an error that names it (exit 1), for the flag and the variable
alike.

## Another config file

`grofe --config <file>` loads `<file>` in place of `config.toml`, and
`GROFE_CONFIG=<file>` does the same. Without a folder option, the file's
folder becomes the config folder: `conf.d/` and the rest are read from next
to the file. A relative path is taken from the current directory and `~/`
from `$HOME`. A file that does not exist loads as empty. `reload` re-reads
the same file, and `show-config` honours both. A folder is refused by
`--config` (exit 1); a `GROFE_CONFIG` naming a folder is ignored with a
notice on stderr.

## Which file and folder win

An empty variable counts as unset. The main file is the first of:

1. `--config <file>`
2. `GROFE_CONFIG`
3. `<folder>/config.toml`

The folder is the first of:

1. `--config-dir <dir>`
2. `GROFE_CONFIG_DIR`
3. the folder of the file named by `--config` or `GROFE_CONFIG`
4. `$XDG_CONFIG_HOME/grofe`, else `~/.config/grofe`

| You pass | Main file | Every other file |
| --- | --- | --- |
| nothing | `~/.config/grofe/config.toml` | `~/.config/grofe/` |
| `--config F` | `F` | the folder of `F` |
| `--config-dir D` | `D/config.toml` | `D/` |
| `--config F --config-dir D` | `F` | `D/` |

So `--config F --config-dir D` reads `F` plus every other layer from `D`.

Only the command that starts the server picks the file and the folder. The
server it starts gets both. A later command with another `--config` prints
`grofe: server uses <path>`, and one with another `--config-dir` prints
`grofe: server uses config folder <dir>`; both carry on with the running
server.

## A small config

```toml
prefix = "C-a"
history-limit = 100000
mouse = true

[status]
right = "#{session_name} %H:%M"
interval = 5

[keys.prefix]
"|" = "split-window -h -c '#{pane_current_path}'"
"-" = "split-window -v -c '#{pane_current_path}'"
"r" = { cmd = "reload", desc = "reload the config" }
```

Every option has a TOML key, listed in the
[options reference](reference/options.md). Most are the tmux name; some live
in a table (`status.right` is `right` under `[status]`) and durations say
their unit (`escape-time-ms`). Booleans are `true` and `false`. A binding's
`desc` shows in `list-keys` and the key menu (`prefix Space`). The default
`history-limit` is 2000 lines, as in tmux.

## Split a config with `include`

`include` loads other TOML files from the one that names it. It takes one
path or a list:

```toml
include = ["keymaps.toml", "theme.toml"]
```

- A relative path is taken from the folder of the file that names it, not
  the current directory.
- The included files apply after the including file's own keys, wherever
  `include` stands in it, and in list order. A later file wins.
- An included file loads at the layer of the file that names it. A file
  included from `config.toml` is still under `conf.d/`, `user.toml` and
  `settings.toml`.
- An included file can include more files. A cycle, such as a file that
  includes itself, is an error.
- A missing file is an error, and so is a value that is not a path or a
  list of paths (`include = 3`). `include = []` includes nothing.
- Errors name the file, the line and the key, like every other config
  error.

`include` is a top-level key, so put it above the first `[table]`. In TOML,
a key below a table header belongs to that table.

`source-file` is the same key under its older name. When a file uses both,
the `include` files apply first and the `source-file` files after them, and
an error names the key that was used.

## Split `init.luau` with `require`

`require("keymaps")` in `init.luau` loads `keymaps.luau` from the config
folder and returns what it returns. A dot in the name is a folder:
`require("ui.status")` loads `ui/status.luau`. The
[Lua page](lua.md#modules-with-require) has the exact rules.

## A tidy layout

One way to split a config, with the TOML and the Lua side by side:

```text
~/.config/grofe/
  config.toml      # options, and the include list
  keymaps.toml     # key bindings
  theme.toml       # the status line
  init.luau        # requires the Lua modules
  keymaps.luau     # keys that run Lua
  ui/
    status.luau    # a git branch widget for the status line
```

`config.toml`:

```toml
include = ["keymaps.toml", "theme.toml"]

prefix = "C-a"
history-limit = 100000
mouse = true
```

`keymaps.toml`:

```toml
[keys.prefix]
"|" = "split-window -h -c '#{pane_current_path}'"
"-" = "split-window -v -c '#{pane_current_path}'"
"r" = { cmd = "reload", desc = "reload the config" }
```

`theme.toml`:

```toml
[status]
position = "top"
style = "bg=colour235,fg=colour250"
left = "#[bold] #{session_name} #[default]"
right = "#{@git} %H:%M"
```

`init.luau`:

```lua
require("keymaps")
local status = require("ui.status")
grofe.log("status widget every " .. status.every_ms .. " ms")
```

`keymaps.luau`:

```lua
grofe.bind("prefix", "g", function()
  grofe.cmd({ "new-window", "-n", "git", "lazygit" })
end, { desc = "Open lazygit" })
```

`ui/status.luau`:

```lua
local M = { every_ms = 5000 }

function M.refresh()
  grofe.system({ "git", "rev-parse", "--abbrev-ref", "HEAD" }, function(r)
    local branch = r.code == 0 and r.stdout:gsub("%s+$", "") or ""
    grofe.cmd({ "set-option", "-g", "@git", branch })
  end)
end

M.refresh()
grofe.every(M.every_ms, M.refresh)
return M
```

`keymaps.toml` and `keymaps.luau` can share a name: `include` names the
file in full, and `require` only looks for `.luau` and `.lua` files. The
same tree works in any folder you pass to `--config-dir`.

## Running as tmux

Started through a `tmux` link (see
[Switching from tmux](switching-from-tmux.md)), with no `-f`, `--config`,
`--config-dir`, `GROFE_CONFIG` or `GROFE_CONFIG_DIR`, grofe looks for tmux's
default config files, in tmux's order:

1. `/etc/tmux.conf`
2. `~/.tmux.conf`
3. `$XDG_CONFIG_HOME/tmux/tmux.conf`, only when that variable is set and
   not empty
4. `~/.config/tmux/tmux.conf`

`~` is `$HOME`, or your home from the user database when `HOME` is unset.
A path that is the same string as an earlier one is checked once, so with
`XDG_CONFIG_HOME=$HOME/.config` that file loads once. Two paths that are one
file through a symlink both load, as in tmux.

When at least one of these exists, the server behaves exactly as if you had
passed each existing file with `-f`. Every one loads, in the order above,
and a later file wins. Parse errors, `reload` and `#{config_files}` work as
they do with `-f`. No TOML loads: not `config.toml`, not `conf.d/`, not
`user.toml`, not `settings.toml`. So `P` in customize-mode saves nothing,
as with `-f`. The files are chosen when the server starts; a file you add
later is read by the next server.

When none of these files exists, the TOML config loads as usual. To get
your TOML config while a tmux file exists, pass `--config` or
`--config-dir` (or set `GROFE_CONFIG` or `GROFE_CONFIG_DIR`). An explicit
`-f` also wins over the defaults. A server started as `grofe` never reads
these files.

## Check, explain, reload

`show-config` needs no server. It prints every setting (`--file <path>`
checks one file); with a setting's name and `--explain` it says which file
set it:

```console
$ grofe show-config status.right --explain
status.right = "#{session_name} %H:%M"
  set by ~/.config/grofe/config.toml
```

A typo is an error naming the file, the line and the setting you probably
meant, never a silently ignored option. `grofe reload` checks every file
first; if any has an error it changes nothing and lists every error with its
file and line.

`list-keys` shows where each binding came from: `default`, a file and line,
or `runtime` for one made with `bind-key` since the server started.

## At runtime

`set-option` and `show-options` work as in tmux, by tmux name:

```sh
grofe set-option -g status-position top
grofe show-options -g status-position
```

Once an option is set at runtime the config no longer changes it, so a
`reload` does not undo it.

## The status line

The status line never forks on redraw. `#(...)` commands in it run on a
timer (`status.interval`, in seconds), one at a time, and their output is
cached. Anything else dynamic, such as a build result, is pushed in by
whatever knows the value, so redraws stay instant.

## The look in the screenshots

The screenshots use this config: a pill status bar on top in the Gruvbox
palette, with the agents badge, and frames whose number and controls get the
same rounded caps. It ships as
[`examples/pills.toml`](https://github.com/termage/grofe/blob/main/examples/pills.toml);
copy it into `~/.config/grofe/conf.d/` (it needs a Nerd font), and on a dark
terminal add
[`examples/pills-dark.toml`](https://github.com/termage/grofe/blob/main/examples/pills-dark.toml)
after it. The colours are user options, so changing one repaints the bar.

```toml
{{#include ../../../examples/pills.toml}}
```
