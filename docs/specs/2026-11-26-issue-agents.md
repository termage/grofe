# bifurc: delegate tracker issues to any agent in grofe

## Goal

- A separate Lua plugin that hands tracker issues to any coding agent (Claude Code, Codex, opencode, aider, goose, a local model, OpenRouter or LiteLLM) in grofe panes, reports progress back to the tracker, lets humans steer, triages new issues and runs scheduled loops; GitHub is the first tracker.

## Decisions

- Trackers are adapters behind one Lua interface; GitHub (issues and Projects v2 boards, through `gh`) ships first; Jira and others are one new adapter file each.
- Agents are profiles in config, not code: any command can be an agent; no agent is built in to the plugin.
- Configuration is global (`<grofe config dir>/bifurc.toml`) and per project (`.grofe/bifurc.toml` at the repo root); project keys override global keys.
- v1 covers delegate, status back, steering, triage and loops.
- The plugin is `bifurc`, in its own repo, installed as a `[[pack]]`; config file `bifurc.toml`.
- grofe core gets the API the plugin needs first (Part A), in three PRs; the plugin (Part B) starts after A1.

## Agent profiles

- Shape follows Zed's `agent_servers` and vibe-kanban's executors: a profile is `command`, `args`, `env`, how the prompt is passed, and how state is known.
- Fields:
  - `command`, `args`: the interactive agent, e.g. `claude`, `codex`, `aider`.
  - `prompt`: `"arg"` (positional), `"flag:<name>"` (e.g. `flag:--message`), `"stdin"`, or `"keys"` (typed into the pane after start).
  - `env`: added environment; this is how models are chosen: `ANTHROPIC_BASE_URL` plus `ANTHROPIC_AUTH_TOKEN` for Claude Code through LiteLLM or Ollama; `OPENAI_BASE_URL` / `AIDER_OPENAI_API_BASE` for OpenAI-compatible endpoints such as OpenRouter; `GOOSE_PROVIDER` for goose. Secrets come from env var names (`env_from = {ANTHROPIC_AUTH_TOKEN = "LITELLM_KEY"}`), never literals in the file.
  - `state`: `"hooks"` (grofe's existing agent setup for claude, codex, gemini), `"notify"` (the agent's notify command calls `grofe agent report`, e.g. aider `--notifications-command`, codex `notify`), or `"screen"` (pane heuristics: idle prompt pattern, spinner pattern, from the profile).
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
- A2 processes and agents (PR 2):
  - `grofe.system` takes `env`, `stdin`, an `on_line` streaming callback, a `timeout_ms` up to 1 hour, and kills the process group on timeout.
  - `agent new <kind> -c <dir> [-e K=V] -- <args...>` prints the new pane id (`-P` like `new-window`).
  - Headless agent records: a `system` job can register as an agent (`kind`, label, state) so triage runs show in the sidebar, grid and tray.
  - `agent-changed` carries `msg`; `grofe.agent.history(pane, n)` returns the last n messages.
  - `pane-created` and `pane-closed` events; `grofe.on(event, fn, {pane = id})` scopes a subscription.
- A3 UI (PR 3):
  - `grofe.choose(items, fn, {title?, preview?})` opens the fuzzy chooser and calls `fn(item)` (nil on cancel).
  - `grofe.input(prompt, fn)` reads a line of text.
  - Callbacks get their context: client, pane and cwd of the key press.
  - Sidebar and grid label formats (`agent-sidebar-format`, `agent-grid-format`) so `#{@issue}` can show.
  - Tray reply: Enter on an agent entry can open `grofe.input` and send the text to the pane (opt-in per entry).
- Later, not needed for v1: per-pack Lua isolation (one VM per pack), a pack test harness and local dev link.

## Part B: the plugin

- Adapter interface (`adapters/<name>.luau`), all async with callbacks: `list(query)`, `get(id)`, `comment(id, text)`, `edit_comment(id, cid, text)`, `comments_since(id, since)`, `set_status(id, status)` with status one of `todo`, `in_progress`, `needs_input`, `in_review`, `done`, and `link(id, url)`.
- Delegate: chooser of issues from the configured query; pick one, and the plugin creates a worktree and branch `<prefix>/<id>-<slug>`, starts the chosen profile with the issue prompt (template in config), tags the pane `@issue=<id>`, records the session in `grofe.store`, and sets `in_progress`.
- Status back: on `agent-changed` for a tagged pane, one status comment per session is edited in place; `blocked` -> `needs_input`; `done` -> `in_review` with the PR link (`gh pr list --head <branch>`); `error` -> comment and tray entry.
- Steering: issue comments starting with the trigger (default `/agent`) from a human are typed into the pane once; tray reply (A3) sends text from grofe.
- Triage: issues matching `triage.query` run the triage profile's headless form; each of labels, assignee, duplicates is `suggest` (comment) or `apply`.
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

## Edge cases

- Empty issue list; issue with no body; unicode titles (ASCII slug, id fallback).
- Worktree or branch exists: reuse if it is the same issue's, else refuse with a tray entry.
- Agent pane closed mid-session: comment says stopped, issue back to `todo`.
- Two grofe servers on the same issue: the stored session plus a marker in the status comment prevent a second start.
- Agent command not found: tray error naming the profile.
- Headless output that is not valid JSON when `output = "json"`: triage comments nothing, tray error.
- Board without the mapped column: status skipped, one tray warning.

## Test list

- A0: e2e, `display-menu` from a Lua timer, then a client `display -p` gets its own reply.
- A1-A3: Rust e2e per API, a positive case and its error case each, on private sockets.
- B: Luau unit tests with a fake adapter and fake profiles for AC2-AC4, AC6, AC7, AC9; e2e with a stub `gh` and a stub agent script on PATH recording calls for AC1, AC3, AC5, AC8, AC11.

## Naming

- `bifurc` (2026-11-26): a bifurcation is where a system splits into branches; each issue splits off into its own branch and worktree. Checked free on GitHub, Homebrew and crates.io.

## Out of scope

- Jira and other adapters beyond the interface.
- Driving agents over ACP (field reserved; a later version can use it for headless jobs and richer status).
- Webhooks or a hosted service; everything polls from the user's machine.
- A cloud sandbox; agents run locally in worktrees.
- Per-pack Lua isolation and a pack test harness (grofe follow-ups).
