# Lua scripting and packs

grofe runs Luau (a sandboxed Lua) inside the server. Use it to bind keys, react to events, run
timers and fill the status line. `grofe pack` installs plugins written in Luau, and tmux plugins too.

## Where your script lives

`<config dir>/init.luau` (or `init.lua` if that is the file you have). It runs when the server
starts and again on every `reload`. Each time, grofe drops the old Lua state first, with all its
key bindings, timers and event subscriptions, so a reload never doubles anything up.

The order is: installed packs in name order, then your `init.luau`. An error in any of them is
logged, shown by `show-messages` with the name of the script, and never stops startup.

## Modules with `require`

Split `init.luau` into modules in your config folder. `require(name)` loads the first file
that exists, in this order (inside a pack, `<config dir>` is the pack's own folder, see
[Writing a plugin](#writing-a-plugin)):

1. `<config dir>/<name>.luau`
2. `<config dir>/<name>.lua`
3. `<config dir>/<name>/init.luau`
4. `<config dir>/<name>/init.lua`

A dot in the name is a folder, so `require("ui.status")` tries `ui/status.luau`, then
`ui/status.lua`, then `ui/status/init.luau` and `ui/status/init.lua`. A leading `./` is allowed
and changes nothing. The call returns what the module returns, or `true` when it returns nothing.

```lua
local status = require("ui.status") -- runs ui/status.luau, returns its table
require("keymaps")                  -- runs keymaps.luau
```

- A module runs once per load. Requiring it again returns the same value without running it.
  Every `reload` starts with an empty cache, so edited modules run again.
- `require` also works in callbacks that run later, such as a key binding or a timer, and in
  the `lua` command, even when the folder has no `init.luau`.
- A required module shares the call's 50 ms budget and memory limit.
- Symlinks are followed. A module linked into the folder from elsewhere (stow, chezmoi) loads.

Each of these is a Lua error, logged like any other:

- an empty name, a name with `..`, or an absolute path (`/etc/passwd`)
- a name with a file extension: write `require("keymaps")`, not `require("keymaps.luau")`
- a missing module: the error names it and lists every path tried
- a cycle, where a module requires itself through other modules
- an error inside the module, which `require` passes on

There is one `require`, and it looks where the running code lives. Your `init.luau`, and
anything it calls later, loads from the config folder; a pack, at load time and in its
callbacks, loads from the pack's own folder, so two packs can each have an `ui.luau`. A pack
does not see your config folder. (Before this change a pack's `require` also read the config
folder; a pack that relied on that must move those modules into its own folder.) Without a config folder, `require` in your own code is an
error.

## The no-freeze contract

Every entry into Lua (the init file, a key, an event, a timer, a `system` callback, the `lua`
command) is one call with its own limits:

- 50 ms of run time. After that every step raises an error, so even a `pcall` loop ends within
  twice the budget.
- 64 MiB of memory.
- No `io`, no `os.execute`, no `loadstring`, no native modules. `require` loads modules from
  your config folder, or from a pack's own folder (see [Modules with `require`](#modules-with-require)).
- Calling the `lua` command from Lua is an error, not a deadlock.

When a call goes over, grofe aborts it, logs `<script>: over its 50 ms budget` and carries on. The
worst case is a stutter of one budget. Anything slow (a `git` call, a network fetch) belongs in
`grofe.system`, which runs outside the server and calls you back with the result.

## The API (`grofe.api == 1`)

Callback ids are small numbers starting at 1. `cmd` raises a Lua error when the command fails, so
wrap it in `pcall` if you expect that.

```lua
grofe.cmd({ "split-window", "-h" })        -- argv form, returns the output
grofe.cmd("new-window -n logs; split-window") -- string form, commands run in order
local s = grofe.fmt("#{session_name}", "%1") -- expand a format, target optional
local id = grofe.bind("prefix", "g", function() grofe.cmd("display-message hi") end,
  { desc = "Say hi", ["repeat"] = false })   -- bind a key, returns an id
grofe.on("window-created", function(e) grofe.log("new window") end)
grofe.on("pane-closed", function(e) grofe.log(e.pane) end, { pane = "%3" }) -- one pane only
grofe.every(2000, function() end)            -- timer, at least 100 ms
grofe.system({ "git", "status" }, function(r) grofe.log(r.stdout) end,
  { cwd = "/tmp", timeout_ms = 5000 })       -- r has code, stdout, stderr
local last = grofe.agent.history("%3", 5)    -- the agent's last 5 messages, oldest first
grofe.cancel(id)                             -- stop a timer, binding, event or process
grofe.log("text")                            -- write to the server log
local cfg = grofe.plugin("my-plugin")        -- what that plugin's init.luau returned
grofe.notify("build finished", { level = "warn", pane = "%3" }) -- a tray entry
local text = grofe.json.encode({ a = { 1, 2 } })  -- '{"a":[1,2]}'
local value = grofe.json.decode(text)             -- back to a table
local conf = grofe.config("tool", "/path/in/a/repo") -- settings files, merged
```

`grofe.every` and `grofe.system` are the only way to run things over time. Event names are the ones
`grofe control` can subscribe to (see [protocol v1](https://github.com/termage/grofe/blob/main/docs/protocol-v1.md)); the callback receives the
event as a table. `pane-created` and `pane-closed` carry the pane id as `pane`; they fire where a
pane enters or leaves the server, in order with the other events of the command: a new pane's
`pane-created` comes before that command's `window-created` and any `agent-changed` for the pane,
and a kill's `pane-closed` after its `pane-exited`. `agent-changed`
carries the report's text as `msg`, which is absent when the report had none. Pass `{ pane = "%3" }` to `grofe.on` to hear
only events of that pane; an unknown pane raises an error, and the subscription ends when the
pane closes.

### Running processes

`grofe.system(argv, fn, opts)` runs `argv` off the server and calls `fn` with
`{ code, stdout, stderr }` when it ends. The options:

- `cwd`: the working directory.
- `env`: a table of variables added to the environment.
- `stdin`: text fed to the process, which is then closed.
- `on_line`: called with each line of stdout as it arrives (the last one even without a
  newline). The result still holds all of stdout.
- `timeout_ms`: at most 3600000 (one hour), default 10000. At the timeout grofe kills the process
  and everything it started (its whole process group) and reports `code = -1`.
- `agent`: `{ kind, label }` shows the job as a `working` row while it runs; see
  [Jobs shown as working rows](agents.md#jobs-shown-as-working-rows). `kind` and `label` are
  required.

A bad option, or a key that is not one of these, raises an error naming it when `system` is called. Output that is not valid UTF-8 is
converted with replacement characters.

`grofe.agent.history(pane, n)` returns the last `n` messages the pane's agent reported, oldest
first. Grofe keeps the last 50 per agent.

### Notifications

`grofe.notify(text, { pane = ..., level = ... })` adds an entry to the notification tray (see
`notify list`). `text` must be a string. `pane` is a pane id such as `"%3"`; without it the
entry belongs to the active pane. A tray entry always belongs to a pane (the tray jumps to it),
so with no pane given and no session, or with an unknown pane, the call raises an error. `level` is `"info"`
(default), `"warn"` or `"error"` and is shown as the entry's state. It never raises a toast or
a desktop message by itself.

### JSON

`grofe.json.encode(value)` returns JSON text. A table holding `1..n` becomes an array, any
other table an object with string keys, and an empty table is `{}`. Functions, userdata and
NaN raise an error, as does a table nested deeper than 64 levels (which also catches a table
that contains itself). A table of more than 100,000 values in total (a subtable shared in
several places counts once per reference), or a sparse array longer than 100,000, also raises an
error, in `grofe.json.encode` and in `grofe.store.set`. `grofe.json.decode(text)` returns the value, or raises on invalid
text; JSON `null` becomes `nil`.

### Settings files

`grofe.config(name, dir)` reads `<config dir>/<name>.toml`, then lays
`<git root of dir>/.grofe/<name>.toml` over it: the project's keys win, and nested tables are
merged key by key. A file that does not exist counts as empty, so the result is always a table.
Without `dir` only the user file is read. A file that does not parse raises an error that names
the file and the line.

## Example: a status widget

The status line never calls Lua. A widget pushes a value into an option, and the format reads it.

```lua
local function refresh()
  grofe.system({ "git", "rev-parse", "--abbrev-ref", "HEAD" }, function(r)
    local branch = r.code == 0 and r.stdout:gsub("%s+$", "") or ""
    grofe.cmd({ "set-option", "-g", "@git", branch })
  end)
end

refresh()
grofe.every(2000, refresh)
```

Show it in the status line with `#{@git}`, for example in `config.toml`:

```toml
[status]
right = "#{@git} #{host_short}"
```

## Example: a layout script on a key

```lua
grofe.bind("prefix", "D", function()
  grofe.cmd({ "new-window", "-n", "dev" })
  grofe.cmd({ "split-window", "-h" })
  grofe.cmd({ "send-keys", "-t", "dev.1", "nvim .", "Enter" })
  grofe.cmd({ "send-keys", "-t", "dev.2", "cargo watch -x test", "Enter" })
end, { desc = "Dev layout" })
```

## grofe pack

Declare plugins in `config.toml`. Cloning never happens inside the server, so a plugin is a table
of data, not a Lua call:

```toml
[[pack]]
src = "github:someone/grofe-git"   # or https://, file:// or an absolute path
rev = "v1.2"                       # optional: branch, tag or commit to track
name = "git"                       # optional: defaults to the repo name
enabled = true                     # optional
```

| Command | What it does |
| --- | --- |
| `grofe pack sync` | clone what is missing and check out the revisions in the lock |
| `grofe pack update [name] [--yes]` | fetch, print the commits each plugin would gain, ask, then write the lock |
| `grofe pack list` | show every plugin and its state |
| `grofe pack clean` | remove clones that no `[[pack]]` declares |

Clones live in `$XDG_DATA_HOME/grofe/pack/<name>`. The lock is `<config dir>/pack-lock.toml`: one
entry per plugin with the source and the exact commit. Commit it to keep machines in step.
`update` without a terminal needs `--yes`. Run `reload` after `sync` to load what was installed.

The kind of plugin comes from the repo. A `grofe-plugin.toml` with `name` and `min_api` marks a
Luau plugin. A repo with `*.tmux` files and no manifest is a tmux plugin: each `*.tmux` runs
through `run-shell` at load, so this replaces TPM.

`min_api` is the oldest plugin API (`grofe.api`) the pack runs on. A grofe whose API is at
least that loads the pack; an older one skips it and says why in `show-messages`. `sync` still
installs such a pack, so it starts working after you upgrade grofe. The earlier `api` key is
still read, as the same number, and an `api` above this grofe's API fails `sync` with the
plugin's name.

A user can override a pack's options on its entry:

```toml
[[pack]]
src = "github:someone/grofe-git"
options = { interval = 5000, theme = { accent = "blue" } }
```

## Writing a plugin

```
my-plugin/
  grofe-plugin.toml   -- name = "my-plugin"  and  min_api = 1
  init.luau
```

```toml
# grofe-plugin.toml
name = "my-plugin"
min_api = 1

[options]            # defaults, read with grofe.options()
interval = 2000
theme = { accent = "green" }
```

`init.luau` runs at load, and what it returns is stored for `grofe.plugin("my-plugin")`:

```lua
local M = { prefix_key = "g" }
grofe.bind("prefix", M.prefix_key, function() grofe.cmd("display-message plugin") end)
return M
```

The user can then read or tune it from their own `init.luau`, which loads after every pack.

What a pack can use besides the API above:

- `require(name)` loads `<name>.luau` and friends from the pack's own folder, never from the
  user's config folder.
- `grofe.pack_dir()` returns the pack's folder, for data files that ship with it.
- `grofe.options()` returns the `[options]` defaults of the manifest with the user's
  `options = {...}` on the `[[pack]]` entry laid over them (nested tables merge key by key).
  Outside a pack it raises an error.
- `grofe.store` keeps data between reloads and restarts, private to the pack, under the state
  directory (`<state dir>/packs/<name>/store.json`). `store.get(key)` returns the value or
  `nil`, `store.set(key, value)` saves a string, number, boolean or table (a function or
  userdata raises an error; `nil` deletes), and `store.delete(key)` removes it. Every call reads
  or writes the file (writes are locked, so two servers can share it), so keep values small. Outside a pack it raises an error.

Inside a pack's files, `require`, `grofe.store`, `grofe.options()`, `grofe.pack_dir()` and the
callbacks you register belong to that pack wherever they are called from, so a function the pack
returns can be called from your `init.luau`.

Errors name their pack. A callback that fails is logged as `pack <name> callback <id>: <error>`.
A pack that fails while loading, whether the error is in its `init.luau`, a `require` or a
`min_api` that is too new, shows up in `show-messages` too, and everything it registered
before the failure (key bindings, timers, event subscriptions) is removed again, so a pack is
never left half loaded. The other packs and your `init.luau` still load.

## Editor setup

Install `luau-lsp` and point it at the declarations so `grofe.*` completes and type-checks.
In a luau-lsp client, map a name to the file in `luau-lsp.types.definitionFiles`:

```json
{ "luau-lsp.types.definitionFiles": { "grofe": "types/grofe.d.luau" } }
```

From the command line the same is `luau-lsp lsp --definitions:@grofe=types/grofe.d.luau`.
