# Coding agents

grofe knows which panes run a coding agent (Claude Code, Codex, Gemini CLI,
opencode, pi, aider, or anything you teach it), what each one is doing, and
which ones need you. It shows that in the status line, a picker, a sidebar and
a grid, and tells you through a notification tray, toasts and desktop
notifications. All of it is built in: no plugin, no session manager and no
helper script is needed.

Everything here is on under grofe's own identity (running `grofe`). Under the
tmux identity (grofe installed as `tmux`) the agent tracking and commands still
work, but the default keys, glyphs and badge are left out so the stock tmux
setup is unchanged. The commands are grofe's own, so call them as `grofe`:
under the `tmux` name they are unknown commands, as they are in tmux.

## Quick start

1. Install the hooks for the agents you use. This edits the agent's own
   config, shows the change and asks first:

   ```sh
   grofe agent setup claude      # also: codex, gemini, opencode
   grofe agent setup --status    # per kind: installed, not installed or agent not found
   ```

2. Start an agent in a grofe pane as usual. To let grofe start it with
   `grofe agent new claude` (see [Starting agents](#starting-agents)).
3. Press `prefix u` to jump to the agent that needs you most.

Agents with no hooks still show up: grofe recognises them by program name and
treats a bell or a desktop notification escape from them as "needs a look".

After upgrading grofe, restart the server (`grofe kill-server`, or `prefix Q`)
so it runs the new build; a running server keeps the build it started with.

## States

Each agent pane has one state:

| State | Glyph | Meaning |
|---|---|---|
| `blocked` | `⏸` | waiting for you: a permission prompt, a question, a login |
| `error` | `✗` | it stopped on an error |
| `working` | `●` | busy |
| `idle` | | started, nothing going on |
| `done` | | finished its turn |

A pane is also **unseen** (`✓`) when it finished or rang a bell while you were
looking elsewhere. Looking at it (the active pane of a focused client) clears
that. Lists are sorted by urgency: blocked, error, unseen, working, idle,
done, oldest first within each.

## Keys

| Key | Command | What it does |
|---|---|---|
| `prefix u` | `agent next` | jump to the most urgent agent |
| `prefix U` | `choose-agent` | the agent picker |
| `prefix b` | `agent-sidebar toggle` | show or hide the sidebar |
| `prefix O` | `agent-grid` | the grid overview |
| `prefix y` | `choose-notification` | the notification tray |
| `prefix Space` | `key-menu -T prefix` | the key menu |
| `prefix M-Space` | `next-layout` | moved here from `prefix Space` |

Rebind any of them like any other key, in TOML or with `bind-key`.

## The key menu

`prefix Space` opens a box titled `Keys · prefix` that lists the bindings of
the prefix table by group (window, pane, session and so on, then your own
groups by name, with `other` last), in as many columns as the client is wide; a narrow client gets one
column. Each group has a bold header, and each binding shows its key in a
bold, right-aligned column and its description. A binding with no
description shows its command, dimmed, and a description too long for its
column ends in `…`. A table too big for the box is split into pages, and the
title shows which page is showing; the page always holds the selected row.

Pressing a key that is bound in the table runs that binding at once and
closes the menu. The menu keeps these keys for itself: `Up`, `Down`, `C-p` and
`C-n` move the selection, `PageUp` and `PageDown` move to the first row of
the previous or next page (a screenful of results while searching), `Home` and
`End` jump to the first and last row, `Enter` runs the selected row when
`Enter` is not bound itself, and `Escape` and `C-c` close it. A binding on one
of those keys, such as `prefix Up` for `select-pane -U`, is still listed and
can be reached by searching. The prefix key itself is ignored while the menu
is open, so it cannot start a prefix sequence under the menu. A binding that
runs `switch-client -T <table>` opens that table's menu. A hint at the bottom
of the box lists the main keys.

`/` or `C-f` opens a search line at the top of the box. When `/` is itself
bound in the table, as it is in the prefix table (`Describe key binding`),
`/` runs that binding and only `C-f` searches; the hint says which. Search is
fuzzy over each binding's key, description, group and command, so `splt`
finds `split-window`. Results are one list, best match first, each row
`key  description  · group` with the matched characters underlined. `Enter`
runs the selected result. `Backspace` on an empty query, or `Escape`, leaves
search and goes back to the groups; `Escape` again closes the menu.

`key-menu -T <table>` opens the menu for any key table. The group comes from
`bind-key -G <group>` or a `group` in a TOML binding, and the description from
`bind-key -N` or `desc`. Both are formats, so a group such as
`Agents (#{agents_attention})` works.

## The picker

`prefix U` opens a list of every agent with its state, time in that state,
`session:window.pane`, kind, last message and directory, and a live preview of
the selected pane below it. Type to filter. `Enter` jumps, `C-x` kills the pane
(it asks first), `C-y` copies the pane's target, `C-s` marks it seen, `Escape`
closes. Sessions in git worktrees of one repo sit under a row named after the repo, each
row showing its branch (see [Worktrees](#worktrees-in-the-sidebar-and-picker)).

Jumping keeps you in your session when the pane's window is in it, and
switches session otherwise.

## The sidebar

`prefix b` shows a strip at the right of your view listing the agents, grouped
by session, two lines each. When panes are framed it sits in a frame like theirs, styled by
`pane-border-style` (`pane-active-border-style` while it has the keys), and the
selected row takes `mode-style`. It is drawn by grofe, not a pane, so it runs
no process. It is per client and hidden until you toggle it (or set
`show-on-attach`), and it hides itself while the terminal is narrower than
`min-client-width`. Each session starts with a header row that shows the most
urgent state of its agents (blocked, then done, then working, then idle). Click
an agent row to jump (a header jumps nowhere); `agent-sidebar focus` gives it the
keys until `Escape`.

A row's label is the option `agent-sidebar-format` (default `#{window_name}`),
expanded for the agent's pane, so `#{@issue}` or any other pane option can show.
A format that does not parse shows as the text it is.

### Worktrees in the sidebar and picker

Sessions whose directory is in a git worktree sit under one header row named
after the main repo's directory, in the sidebar and in the picker, and each row
shows its branch, then its label (`agent-sidebar-format`, so `#{@issue}` shows). A session in the main checkout joins
them when its repo has a worktree session. A plain repo, or a directory outside
git, keeps its own rows. Grofe finds the repo from the worktree's `.git` file
and `commondir`, as `git rev-parse --git-common-dir` does, and remembers it per
directory; only the branch is read again, so a `git checkout` shows at once. A
directory that becomes a repo later is seen after a restart.

## The grid

`prefix O` shows every agent at once, one framed tile each. The title sits in
the top edge (glyph, `session:window.pane`, kind, time in state and last
message, cut with `…` when it does not fit) and the box holds the bottom
`preview-lines` lines of its pane, clipped to the tile. Frames use the
window's `pane-border-lines` and `pane-border-style`; the selected tile takes
`pane-active-border-style`. Tiles sit in as many columns of `tile-width` as
fit, one cell apart and widened to fill the screen, so none is cut at the
edge; rows scroll to keep the selection in view. It is read only: arrows
move, typing filters, `Enter` jumps, `Escape` closes. The label after the glyph
is the option `agent-grid-format` (default `#{session_name}:#{window_index}.#{pane_index}`),
expanded for the agent's pane.

## Notifications

A notification is raised when an agent enters a state listed in
`notify.on` (by default `blocked`, `error` and `done`), unless you are already
looking at it. A `done` after less than `done-min-work` of work is not worth
a notification and is skipped.

- **Tray:** the last `history` notifications (default 10). `prefix y` opens
  it: `Enter` jumps, `C-x` dismisses one, `C-l` clears all. From the shell:
  `grofe notify list`, `grofe notify dismiss <id>`, `grofe notify clear`.
  An entry a script posted with `grofe.notify(text, { pane = ..., reply = true })`
  asks for a line on `Enter` instead of jumping, and types it into that agent
  as `agent prompt` does (control characters removed, that pane only); `Escape`
  sends nothing.
- **Status badge:** the default status line shows a badge such as `⏸2 ✓1`
  when something needs you; click it to open the tray. Window tabs get the
  agent glyph, and a blocked pane's border turns yellow.
- **Toasts:** a short message in a corner, by default for `blocked` only.
- **Desktop:** by default for `blocked`, `error` and `done`. With
  `delivery = "auto"` grofe sends OSC 9 to your terminal when it knows the
  terminal is not focused (Ghostty, WezTerm, iTerm2 and kitty show it as a
  desktop notification, over ssh too), and runs `notify-send` (Linux) or
  `osascript` (macOS) when no client is attached. A server started with
  `GROFE_NOTIFY_DESKTOP=off` in its environment never runs those built-in
  notifiers (a configured `command` still runs); grofe's tests and CI use it.
- **Mute:** `grofe notify mute [duration]` sends everything to the tray only
  until `grofe notify unmute` or the duration ends; the badge shows `🔕`.
  `notify.enabled = false` turns notifications off altogether.

"Not focused" needs `focus-events` on; with it off grofe assumes you are
looking.

## Starting agents

```sh
grofe agent new claude                 # new window, current directory
grofe agent new codex -p               # split the current pane instead
grofe agent new claude -w -b fix/api   # in a new git worktree on branch fix/api
```

Each built-in kind starts the program of its own name. To run something else,
set `command` for the kind:

```toml
[agents.kinds.claude]
command = "claude"
```

Options: `-c dir` (default: the current pane's directory), `-n name` (the
window name), `-d` (do not switch to it), `-b branch`, `-w` always use a new
worktree, `-W` never. Without `-w` or `-W`, the kind's own `worktree` key or
else `agents.new.worktree` decides (`never` by default, or `always`, or `ask`,
which prompts for a branch; an empty answer means no worktree). A worktree
needs a branch: from the shell, pass `-b`.

Scripts can also pass `-P` (print the new pane's id, like `new-window -P`),
`-e K=V` (a variable for the agent's environment, repeatable; a word without
`=` is an error) and a command after `--`, which runs instead of the kind's
own:

```sh
id=$(grofe agent new claude -P -c ~/work/api -e ISSUE=42 -- claude --model opus)
```

Worktrees go to `<repo>/../worktrees/<branch>`, with any `/` in the branch
turned into `-`. Set `agents.new.worktree-command` to use another tool (for
example worktrunk); when its last line of output is an existing directory,
that directory is used.

## Jobs shown as working rows

A Lua script can make a background job show up next to your agents:
`grofe.system(argv, fn, { agent = { kind = "triage", label = "issue 7" } })`
(see Running processes in the Lua scripting chapter). `kind` and `label` are
required. A job is not an agent: it has no pane and no record in the agent
store, so no `agent-changed` event, hook, `agent next` or attention count
involves it, and `agent list` leaves it out. While it runs it is one `working`
row in the sidebar and the grid; Enter on the row does nothing. Two jobs with
the same label are two rows.

The row lives only while that job runs. When the process exits (0 is `done`,
anything else `error`), times out, is cancelled with `grofe.cancel`, or the
config is reloaded, the row goes. On exit or timeout the tray gets a `done` or
`error` entry under the job's label. Only `notifications.enabled` gates it:
`on` and `done-min-work` are about an agent's states and do not apply, and with
no pane there is no seen check, rate limit or desktop message. An `error` entry
says why: `exit N`, `timed out after Nms`, or the text of the failed spawn.

## How grofe learns an agent's state

- **Hooks** (`grofe agent setup <kind>`): the agent calls
  `grofe agent report <state> --hook <kind>` on its own events. Hook mode
  reads the hook payload from stdin, never blocks the agent, and always exits 0.
  For `claude`, `codex` and `gemini` the setup merges hooks into the agent's
  JSON file (`$CLAUDE_CONFIG_DIR/settings.json` or `~/.claude/settings.json`,
  `$CODEX_HOME/hooks.json` or `~/.codex/hooks.json`,
  `~/.gemini/settings.json`). It edits the file in place (a symlinked
  dotfile stays a symlink) and keeps a one-time backup next to it
  (`<file>.grofe-bak`). For `opencode` it writes a plugin,
  `~/.config/opencode/plugins/grofe.js`. `pi` needs no hooks; grofe
  recognises it by name. `--dry-run` shows the change and writes nothing,
  `--yes` skips the question, and `--remove` takes out exactly what it
  added. A Claude Code started through its SDK or `claude -p` (a memory
  hook's helper, say) inside a pane is not that pane's agent: its hook
  reports are ignored, so it cannot replace the pane's state or the
  session grofe resumes there.
- **Reports from anything:** a script can call
  `grofe agent report <state> [--kind k] [--msg text] [--blocked permission|question|auth]`
  from inside a grofe pane. Outside one, or with no server, it does nothing.
- **OSC 7501**, for programs that speak it:
  `ESC ] 7501 ; state=blocked:kind=permission:msg=<base64> ST`.
- **By program name:** the foreground program is matched against each kind's
  `match` list (looking through `node`, `npx`, `uv` and similar launchers). A
  bell or an OSC 9/777 notification from an agent that sends no reports marks
  it unseen.

- **The screen:** for a pane that has never had a report, grofe reads the last
  `lines` lines of the screen once its output rests, and matches them against
  the kind's detection file, `working` first, then `blocked`, then `idle`. A
  screen that matches nothing changes nothing, except that a `working` or
  `blocked` state the screen (or `agent prompt`) gave falls back to `idle`. Files for `claude`, `codex`,
  `gemini`, `opencode` and `aider` are built in and only match obvious prompts.
  `<config dir>/agents/<kind>.toml` replaces the built-in file for that kind:

  ```toml
  lines = 20                       # default
  working = "esc to interrupt"     # a regex, or an array of them
  blocked = ["Allow\\?", "\\[y/n\\]"]
  idle = "^> $"
  ```

  A file with an invalid regex is skipped, the built-in file stays, and
  `show-messages` names the file and key. A report, hook or OSC 7501 state
  always beats the screen.

`grofe agent explain <pane>` prints the kind, the state and where it came from
(`report`, `osc`, `screen` with the file and the rule that matched, `prompt` for the
`working` that `agent prompt` set, or `bell`).

## Prompting and reading agents

- `grofe agent prompt <pane> [--wait] [--until idle,blocked,done] [--timeout <s>] -- <text>`
  types the text and Enter into an agent pane, and into that pane only. The text
  is untrusted, so control characters other than newline and tab are removed.
  A pane with bracketed paste on gets the text as one paste; otherwise newlines
  and tabs become spaces. Once the text is typed the agent is `working`
  (`agent-changed` fires; `agent explain` says `source: prompt`). For a pane
  whose kind has a detection file and that has had no report, that `working`
  ends as `idle` once the pane has printed output since the prompt and then
  been quiet for one second with no `working` or `blocked` rule matching (a
  matching rule sets the state as usual). A pane grofe cannot read this way
  stays `working` until it reports. With `--wait` it waits for the
  agent's next change to one of the `--until` states (default
  `idle,blocked,done`; any of `idle`, `working`, `blocked`, `done`, `error`) and
  prints it. Exit 1 is a timeout (default one hour), exit 2 a pane that
  closed or an agent that left its pane. A pane that is not an agent, takes no
  input, or an unknown `--until` state, is an error and types nothing.
- `grofe agent read <pane> [-n <lines>]` prints the last lines of a pane (50 by
  default), from the alternate screen when the program is on it, else
  scrollback and screen. `-n 0` prints nothing.
- In Lua: `grofe.agent.prompt(pane, text, { ["until"] = "blocked", timeout_ms = 60000 }, fn)`
  calls `fn(state)` when the state is reached, or `fn()` on a timeout, a
  closed pane or an agent that left; `grofe.agent.read(pane, n)` returns the
  lines joined by `\n` (50 lines without `n`).

Agent text is shown as text: control characters are removed, and it can never
run a command or start a status line style.

## Configuration

All of this is set in your TOML config. The defaults:

```toml
[agents]
enabled = true            # false: no records, no surfaces
scan-interval = "1s"      # how often a busy pane's program is checked

[agents.kinds.claude]     # merges over the built-in kind; enabled = false removes it
match = ["claude"]
resume = "claude --resume {id}"
# command = "claude"      # default: the kind's own name
# worktree = "ask"        # overrides agents.new.worktree for this kind

[agents.new]
worktree = "never"        # ask | always | never
worktree-command = ["git", "-C", "{dir}", "worktree", "add", "-b", "{branch}", "{path}"]

[agents.picker]
width = "80%"
height = "70%"

[agents.sidebar]
width = 32
show-on-attach = false
min-client-width = 120

[agents.grid]
tile-width = 60
preview-lines = 12

[notify]
enabled = true
on = ["blocked", "error", "done"]
toast = ["blocked"]
desktop = ["blocked", "error", "done"]
history = 10
skip-when-seen = true
done-min-work = "30s"
min-interval = "10s"
toast-time = "5s"
delivery = "auto"         # auto | terminal | command | off
sound = false
# command = ["my-notifier", "--", "{title}", "{message}"]
```

A custom `notify.command` is an argv, never run through a shell; `{title}`,
`{message}` and `{pane}` are each replaced inside one element. Put `--` before
them so agent text is never read as an option.

Built-in kinds: `claude`, `codex`, `opencode`, `pi`, `gemini` and `aider`.
Each matches its own name. Three come with a resume command:
`claude --resume {id}`, `codex resume {id}` and `opencode --session {id}`.
Add your own kind with an `[agents.kinds.<name>]` table. `agents.picker`
sizes are percentages written as strings (`"80%"`); the sidebar and grid
sizes are plain numbers of cells.

The look comes from options (see the [options reference](https://termage.github.io/grofe/docs/reference/options.html)):
`agent-glyph-blocked`, `agent-glyph-error`, `agent-glyph-unseen`,
`agent-glyph-working`, `notify-muted-glyph` and `agent-blocked-border-style`.

## Formats

For your own status line or scripts:

| Variable | Value |
|---|---|
| `agent_kind`, `agent_state`, `agent_msg`, `agent_blocked_kind` | the pane's agent; empty when none |
| `agent_since` | seconds in the current state |
| `agent_unseen`, `agent_attention` | `1` or `0` |
| `window_agent_state`, `session_agent_state` | the most urgent state in the window or session |
| `agents_total`, `agents_blocked`, `agents_working`, `agents_attention` | counts |
| `notifications_unread` | count |
| `agents_badge` | for example `⏸2 ✓1`; empty when nothing needs you |

`grofe agent list -F '#{agent_kind} #{agent_state} #{agent_msg}'` prints one
line per agent, most urgent first.
