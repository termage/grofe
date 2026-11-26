# bifurc: delegate tracker issues to any agent in grofe

## Goal

- A separate Lua plugin that hands tracker issues to any coding agent (Claude Code, Codex, opencode, aider, goose, a local model, OpenRouter or LiteLLM) in grofe panes, reports progress back to the tracker, lets humans steer, triages new issues and runs scheduled loops; GitHub is the first tracker.

## Decisions

- Trackers are adapters behind one Lua interface; GitHub (issues and Projects v2 boards, through `gh`) ships first; Jira and others are one new adapter file each.
- Agents are profiles in config, not code: any command can be an agent; no agent is built in to the plugin.
- Configuration is global (`<grofe config dir>/bifurc.toml`) and per project (`.grofe/bifurc.toml` at the repo root); project keys override global keys.
- v1 covers delegate, status back, steering, triage and loops.
- The plugin is `bifurc`, in its own repo, installed as a `[[pack]]`; config file `bifurc.toml`.
- grofe core gets the API the plugin needs first (Part A), in four PRs (A2 splits into A2a processes and events, A2b agent control and detection); the plugin (Part B) starts after A3 (2026-11-26).
- The plugin repo is `termage/bifurc`, private for now (2026-11-26).
- 2026-11-26 ruling: headless jobs are their own list, one entry per running job, shown as working rows in sidebar and grid (not `agent list`), not agents: no agent-changed events, hooks, agent next or attention counts; finish posts a done/error tray entry.
- 2026-11-26 ruling: pane-created and pane-closed are emitted where panes enter and leave the model, not by snapshot diffing.
- 2026-11-26 ruling: a shown `done` pane stays `done` with `agent_unseen` cleared; it does not turn `idle`.
- 2026-11-26 ruling: the sidebar gets one header row per session with the most urgent state of its agent panes (blocked > done > working > idle); no window rows.
- 2026-11-26 ruling: screen detection falls back to `idle`: when a pane's state came from the screen and is `working` or `blocked`, and the screen now matches no rule, it becomes `idle`. Idle rules stay optional, so the built-in files need none.
- 2026-11-26 ruling: `grofe.agent.read(pane, n)` returns one string, lines joined by `\n`; `agent read` prints the same.
- 2026-11-26 ruling: `agent prompt` types the text and Enter through the send-keys path; `--wait` waits on agent state changes, not by polling; exit 0 prints the state, 1 is a timeout, 2 a closed pane; a non-agent pane or an unknown `--until` state is an error (exit 1) naming it. `grofe.agent.prompt(pane, text, {until?, timeout_ms?}, fn(state))` is async over the generation-checked callback path.
- 2026-11-26 ruling: prompt text is untrusted (Part B sends issue comments). `agent prompt` and `grofe.agent.prompt` strip C0 and C1 control characters except newline and tab, then write to the target pane only (never its synchronize-panes targets), and only to a pane that takes input (else an error naming the pane): a pane with bracketed paste on gets one bracketed paste and Enter, any other pane gets newlines and tabs as spaces and Enter. A `Clear` report (agent left, pane open) ends waits with "agent left pane %N", exit 2; "pane %N closed" is for a gone pane. This supersedes the send-keys path above.
- 2026-11-26 ruling: detection files are TOML with `working`, `blocked`, `idle` (a regex string or an array of them) and `lines` (default 20); the built-in files for claude, codex, gemini, opencode and aider are embedded in the binary and conservative; `<config dir>/agents/<kind>.toml` replaces the built-in one; an invalid regex keeps the built-in file and reports file and key in `show-messages`; screen detection applies only to a record that has had no report.
- 2026-11-26 ruling: `agent explain <pane>` prints kind, state and source (`report`, `osc`, `screen`, `bell`), and for `screen` the file and the matching rule.
- 2026-11-26 ruling: resume after a restore needs no change; its tests pass today.
- 2026-11-27 ruling: `agent prompt` marks the agent `working` once the text is typed, through the normal agent state path (`agent-changed` fires, the sidebar updates), before any wait starts; a wait then settles on the next real change to one of the `--until` states, and `idle` after `working` counts as a change. Screen detection does not read output from before the prompt, and a read within 300 ms of the typing cannot end the work with `idle`. While that prompt-set `working` state is pending for a screen-detected pane (no fresher report), the screen is re-read every 500 ms even without new output, until the state leaves `working` or the pane gets a report.
- Herdr-style additions (2026-11-26): `agent prompt --wait` and `agent read`, per-kind detection files with `agent explain`, worktrees grouped under their repo, the unseen `done` state rolled up, and agents resumed after a restore. Herdr's mobile app and plugin marketplace are not copied.

## Agent profiles

- Shape follows Zed's `agent_servers` and vibe-kanban's executors: a profile is `command`, `args`, `env`, how the prompt is passed, and how state is known.
- Fields:
  - `command`, `args`: the interactive agent, e.g. `claude`, `codex`, `aider`.
  - `prompt`: `"arg"` (positional), `"flag:<name>"` (e.g. `flag:--message`), `"stdin"`, or `"keys"` (typed into the pane after start).
  - `env`: added environment; this is how models are chosen: `ANTHROPIC_BASE_URL` plus `ANTHROPIC_AUTH_TOKEN` for Claude Code through LiteLLM or Ollama; `OPENAI_BASE_URL` / `AIDER_OPENAI_API_BASE` for OpenAI-compatible endpoints such as OpenRouter; `GOOSE_PROVIDER` for goose. Secrets come from env var names (`env_from = {ANTHROPIC_AUTH_TOKEN = "LITELLM_KEY"}`), never literals in the file.
  - `state`: `"hooks"` (grofe's existing agent setup for claude, codex, gemini), `"notify"` (the agent's notify command calls `grofe agent report`, e.g. aider `--notifications-command`, codex `notify`), or `"screen"` (the kind's detection file, see A2; a profile may name its own `detect` file).
  - `headless`: the one-shot form for triage, e.g. `claude -p --output-format stream-json`, `codex exec --json`, `opencode run --format json`, `goose run --output-format json`, with `output = "text" | "json"`.
  - `acp`: optional ACP command (e.g. `opencode acp`, `claude-agent-acp`) used for headless jobs instead of `headless`; out of scope for v1 beyond the field (see Out of scope).
- The plugin ships example profiles for claude, codex, opencode, aider, goose, and "claude via LiteLLM", "aider via OpenRouter", "Ollama local".
- `default_agent` and per-action overrides (`delegate.agent`, `triage.agent`, per loop `agent`).

## Part A: grofe core (termage/grofe)

Audit of src/lua, src/server/lua_host.rs, src/server/pack_load.rs, src/server/agents (2026-11-26).

- A0 bug fix, own PR first: commands that answer later (`display-menu`, `agent new -w`, `run-shell`) run from a Lua timer or event leave `state.deferred_reply` set (src/server/mod.rs:510), so the next client request takes it as its reply.
- A1 runtime and packs (PR 1, unblocks Part B):
  - `grofe.notify(text, {pane?, level?})` posts a tray entry.
  - `grofe.json.encode/decode`.
  - `grofe.config(name, dir?)`: `<config dir>/<name>.toml` merged with `<git root of dir>/.grofe/<name>.toml`; missing is empty; parse errors name file and line.
  - Pack-relative `require` and `grofe.pack_dir()`.
  - `grofe.store`: a durable per-pack key-value store (survives reload and restart) for sessions and comment ids.
  - Per-pack error attribution: errors carry the pack name; pack load errors go to `show-messages`; a pack that fails to load is unloaded, not half-registered.
  - Manifest: `min_api` instead of an exact `api` match, and `[options]` defaults readable from Lua.
- A2 processes and agents (PRs 2 and 3; A2a is the first five bullets, A2b the rest):
  - `grofe.system` takes `env`, `stdin`, an `on_line` streaming callback, a `timeout_ms` up to 1 hour, and kills the process group on timeout.
  - `agent new <kind> -c <dir> [-e K=V] -- <args...>` prints the new pane id (`-P` like `new-window`).
  - Headless jobs: a `system` job can show as a `working` row (`kind`, label) in the sidebar, grid and tray while it runs (see the 2026-11-26 ruling).
  - `agent-changed` carries `msg`; `grofe.agent.history(pane, n)` returns the last n messages.
  - `pane-created` and `pane-closed` events; `grofe.on(event, fn, {pane = id})` scopes a subscription.
  - `agent prompt <pane> [--wait] [--until idle,blocked,done] [--timeout <s>] -- <text>` types the text and Enter into the pane; with `--wait` it returns when the agent reaches one of the `--until` states (default `idle,blocked,done`) and prints that state; exit 1 on timeout, exit 2 when the pane closes. Lua: `grofe.agent.prompt(pane, text, {until?, timeout_ms?}, fn(state))`.
  - `agent read <pane> [-n <lines>]` prints the last n lines of the pane, from the alternate screen when the agent uses one, otherwise visible screen plus scrollback. Lua: `grofe.agent.read(pane, n)`.
  - Detection files: `<config dir>/agents/<kind>.toml` with `working`, `blocked` and `idle` regexes matched on the last `lines` (default 20) of the screen, checked in that order; grofe ships files for claude, codex, gemini, opencode and aider; a user file replaces the built-in one for that kind; hooks and reports still win over screen detection.
  - `agent explain <pane>` prints the kind, the state, its source (`hooks`, `report`, `osc`, `screen`, `bell`), and for `screen` the file and the rule that matched.
  - Unseen `done`: `done` stays `done` and counts as unseen until a client shows the pane (exists today); the sidebar has one header row per session showing the most urgent state of its agent panes (`blocked` > `done` > `working` > `idle`); windows have no rows, their roll-up is the `window_agent_state` format. Tests prove both; only the header row is new code.
  - Resume after restore: a restored agent pane runs its kind's resume command with the reported session id (exists today in src/server/restore.rs). An e2e test proves it for a fake kind; gaps found are fixed here.
- A3 UI (PR 4):
  - `grofe.choose(items, fn, {title?, preview?})` opens the fuzzy chooser and calls `fn(item)` (nil on cancel).
  - `grofe.input(prompt, fn)` reads a line of text.
  - Callbacks get their context: client, pane and cwd of the key press.
  - Sidebar and grid label formats (`agent-sidebar-format`, `agent-grid-format`) so `#{@issue}` can show.
  - Tray reply: Enter on an agent entry can open `grofe.input` and send the text to the pane (opt-in per entry).
  - Worktree grouping: sidebar and picker rows of sessions whose cwd is a git worktree sit under their main repo (by git common dir), with the branch as the row label.
- Later, not needed for v1: per-pack Lua isolation (one VM per pack), a pack test harness and local dev link.

## Part B: the plugin

- Adapter interface (`adapters/<name>.luau`), all async with callbacks: `list(query)`, `get(id)`, `comment(id, text)`, `edit_comment(id, cid, text)`, `comments_since(id, since)`, `set_status(id, status)` with status one of `todo`, `in_progress`, `needs_input`, `in_review`, `done`, and `link(id, url)`.
- Delegate: chooser of issues from the configured query; pick one, and the plugin creates a worktree and branch `<prefix>/<id>-<slug>` with `git worktree add` (or `wt` when `delegate.worktree = "wt"`), starts the chosen profile with the issue prompt (template in config), tags the pane `@issue=<id>`, records the session in `grofe.store`, and sets `in_progress`.
- Status back: on `agent-changed` for a tagged pane, one status comment per session is edited in place; `blocked` -> `needs_input`; `done` -> `in_review` with the PR link (`gh pr list --head <branch>`); `error` -> comment and tray entry.
- Steering: issue comments starting with the trigger (default `/agent`) from a human are typed into the pane once; tray reply (A3) sends text from grofe.
- Triage: issues matching `triage.query` run the triage profile's headless form; each of labels, assignee, duplicates is `suggest` (comment) or `apply`.
- Status back waits with `grofe.agent.prompt`/`agent-changed` and reads the agent's last output with `grofe.agent.read` for the final comment.
- Loops: `[[loop]]` with `every`, `query`, `action = "delegate" | "triage"`, optional `agent`; never a second session for an issue; `max_sessions` cap.

## Acceptance criteria

- AC1 A global config naming the GitHub adapter and a repo lists that repo's open issues in the chooser.
- AC2 A project config overrides any global key for panes in that repo.
- AC3 Delegating starts the chosen profile in a new worktree with the templated prompt, tags the pane, and sets `in_progress`.
- AC4 Any profile works: a profile with `state = "notify"` and one with `state = "screen"` both drive the status comment, with no plugin code per agent.
- AC5 A profile's `env` and `env_from` reach the agent process; a literal secret in the config is refused with an error naming the key.
- AC6 Agent state changes edit one status comment; `blocked` -> `needs_input`; `done` -> `in_review` with the PR link.
- AC7 A trigger comment from a human reaches the pane once; other comments and the plugin's own do not.
- AC8 Triage `suggest` only comments, `apply` sets the fields; the run shows as a headless agent while it runs.
- AC9 A loop delegates on schedule, skips issues with a live session, respects `max_sessions`.
- AC10 A new tracker adapter is one file loaded by name from config.
- AC11 `gh` and agent errors show in the tray with the plugin's name and never stop the pack; sessions survive `grofe reload`.
- AC12 `agent prompt --wait` returns the state the agent reached, exits 1 on timeout and 2 when the pane closes; `agent read` returns the agent's last lines, alternate screen included.
- AC13 A user detection file for a kind changes that kind's screen state, and `agent explain` names the file and rule.
- AC14 A session with one `blocked` pane shows `blocked` on its sidebar row; a `done` pane stops counting as unseen once shown.
- AC15 After a restore, an agent pane with a resume command and session id runs that command.
- AC16 Two worktrees of one repo show grouped under that repo in the sidebar.

## Edge cases

- Empty issue list; issue with no body; unicode titles (ASCII slug, id fallback).
- Worktree or branch exists: reuse if it is the same issue's, else refuse with a tray entry.
- Agent pane closed mid-session: comment says stopped, issue back to `todo`.
- Two grofe servers on the same issue: the stored session plus a marker in the status comment prevent a second start.
- Agent command not found: tray error naming the profile.
- Headless output that is not valid JSON when `output = "json"`: triage comments nothing, tray error.
- Board without the mapped column: status skipped, one tray warning.
- `agent prompt` to a pane that is not an agent: error, exit 1; `--until` with an unknown state: error naming it.
- Detection file with an invalid regex: the built-in file stays in use, error in `show-messages` naming file and key.
- `agent read -n 0` prints nothing; a huge `-n` is capped at the pane's history.

## Test list

- A0: e2e, `display-menu` from a Lua timer, then a client `display -p` gets its own reply.
- A1-A3: Rust e2e per API, a positive case and its error case each, on private sockets.
- AC12-AC16: Rust e2e with a fake agent script that prints a prompt, a spinner and an approval line on cue.
- B: Luau unit tests with a fake adapter and fake profiles for AC2-AC4, AC6, AC7, AC9; e2e with a stub `gh` and a stub agent script on PATH recording calls for AC1, AC3, AC5, AC8, AC11.

## Naming

- `bifurc` (2026-11-26): a bifurcation is where a system splits into branches; each issue splits off into its own branch and worktree. Checked free on GitHub, Homebrew and crates.io.

## Out of scope

- Jira and other adapters beyond the interface.
- Driving agents over ACP (field reserved; a later version can use it for headless jobs and richer status).
- Webhooks or a hosted service; everything polls from the user's machine.
- A cloud sandbox; agents run locally in worktrees.
- Per-pack Lua isolation and a pack test harness (grofe follow-ups).
- A mobile app and a plugin marketplace (herdr has both; not wanted).
- Fetching detection files from the network; they ship with grofe or come from the user.
