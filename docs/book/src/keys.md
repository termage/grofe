# Keys

<!-- Generated from the default bindings in src/server/input/defaults.rs.
     Do not edit by hand: run `just docs` to regenerate. -->

Every default binding of the prefix table, as grofe binds it, by the
section the [key menu](agents.md#the-key-menu) files it under. Press the
prefix key (`Ctrl-b`) first. The last column says how tmux's own default for
that key compares: the same command, grofe's own key, or what tmux binds
there instead. Under tmux identity (grofe run as `tmux`, or
`compat-identity = "tmux"`) the keys are tmux's, with tmux's notes.

## window

| Key | Does | Command | tmux |
|---|---|---|---|
| `&` | Kill current window | `confirm-before -p "kill-window #{window_name}? (y/n)" kill-window` | same |
| `'` | Prompt for window index to select | `command-prompt -T window-target -p index "select-window -t \":%%\""` | same |
| `,` | Rename current window | `command-prompt -I "#W" "rename-window \"%%\""` | same |
| `.` | Move the current window | `command-prompt -T target "move-window -t \"%%\""` | same |
| `0` | Select window 0 | `select-window -t :0` | same |
| `1` | Select window 1 | `select-window -t :1` | same |
| `2` | Select window 2 | `select-window -t :2` | same |
| `3` | Select window 3 | `select-window -t :3` | same |
| `4` | Select window 4 | `select-window -t :4` | same |
| `5` | Select window 5 | `select-window -t :5` | same |
| `6` | Select window 6 | `select-window -t :6` | same |
| `7` | Select window 7 | `select-window -t :7` | same |
| `8` | Select window 8 | `select-window -t :8` | same |
| `9` | Select window 9 | `select-window -t :9` | same |
| `M-n` | Select the next window with an alert | `next-window -a` | same |
| `M-p` | Select the previous window with an alert | `previous-window -a` | same |
| `c` | Create a new window | `new-window -c "#{pane_current_path}"` | same |
| `i` | Display window information | `display-message` | same |
| `l` | Select the previously current window | `last-window` | same |
| `n` | Select the next window | `next-window` | same |
| `p` | Select the previous window | `previous-window` | same |
| `w` | Choose a window from a list | `choose-tree -Zw` | same |

## pane

| Key | Does | Command | tmux |
|---|---|---|---|
| `!` | Break pane to a new window | `break-pane` | same |
| `"` | Split window vertically | `split-window -c "#{pane_current_path}"` | same |
| `%` | Split window horizontally | `split-window -h -c "#{pane_current_path}"` | same |
| `;` | Move to the previously active pane | `last-pane` | same |
| `C-Down` | Resize the pane down | `resize-pane -D` | same |
| `C-Left` | Resize the pane left | `resize-pane -L` | same |
| `C-Right` | Resize the pane right | `resize-pane -R` | same |
| `C-Up` | Resize the pane up | `resize-pane -U` | same |
| `C-o` | Rotate through the panes | `rotate-window` | same |
| `Down` | Select the pane below the active pane | `select-pane -D` | same |
| `F` | Float or unfloat the active pane | `if-shell -F "#{pane_floating_flag}" unfloat-pane float-pane` | grofe's own |
| `Left` | Select the pane to the left of the active pane | `select-pane -L` | same |
| `M-Down` | Resize the pane down by 5 | `resize-pane -D 5` | same |
| `M-Left` | Resize the pane left by 5 | `resize-pane -L 5` | same |
| `M-Right` | Resize the pane right by 5 | `resize-pane -R 5` | same |
| `M-Up` | Resize the pane up by 5 | `resize-pane -U 5` | same |
| `M-o` | Rotate through the panes in reverse | `rotate-window -D` | same |
| `Right` | Select the pane to the right of the active pane | `select-pane -R` | same |
| `Up` | Select the pane above the active pane | `select-pane -U` | same |
| `f` | Float or unfloat the active pane | `if-shell -F "#{pane_floating_flag}" unfloat-pane float-pane` | tmux: Search for a pane |
| `o` | Select the next pane | `select-pane -t :.+` | same |
| `q` | Display pane numbers | `display-panes` | same |
| `x` | Kill the active pane | `confirm-before -p "kill-pane #{pane_index}? (y/n)" kill-pane` | same |
| `z` | Zoom the active pane | `resize-pane -Z` | same |
| `{` | Swap the active pane with the pane above | `swap-pane -U` | same |
| `}` | Swap the active pane with the pane below | `swap-pane -D` | same |

## session

| Key | Does | Command | tmux |
|---|---|---|---|
| `$` | Rename current session | `command-prompt -I "#S" "rename-session \"%%\""` | same |
| `(` | Switch to the previous session | `switch-client -p` | same |
| `)` | Switch to the next session | `switch-client -n` | same |
| `L` | Switch to the last session | `switch-client -l` | same |
| `s` | Choose a session from a list | `choose-tree -Zs` | same |

## scratchpads

| Key | Does | Command | tmux |
|---|---|---|---|
| `A` | Choose a scratchpad from a list | `choose-scratchpad` | grofe's own |
| `S` | Hide or show a scratchpad | `toggle-scratchpad` | grofe's own |
| `a` | Open a scratchpad | `new-scratchpad` | grofe's own |

## layout

| Key | Does | Command | tmux |
|---|---|---|---|
| `E` | Spread panes out evenly | `select-layout -E` | same |
| `M-1` | Set the even-horizontal layout | `select-layout even-horizontal` | same |
| `M-2` | Set the even-vertical layout | `select-layout even-vertical` | same |
| `M-3` | Set the main-horizontal layout | `select-layout main-horizontal` | same |
| `M-4` | Set the main-vertical layout | `select-layout main-vertical` | same |
| `M-5` | Select the tiled layout | `select-layout tiled` | same |
| `M-6` | Set the main-horizontal-mirrored layout | `select-layout main-horizontal-mirrored` | same |
| `M-7` | Set the main-vertical-mirrored layout | `select-layout main-vertical-mirrored` | same |
| `M-Space` | Select next layout | `next-layout` | grofe's own |

## agents

| Key | Does | Command | tmux |
|---|---|---|---|
| `O` | Show all agents | `agent-grid` | grofe's own |
| `U` | Pick an agent | `choose-agent` | grofe's own |
| `b` | Toggle the agent sidebar | `agent-sidebar toggle` | grofe's own |
| `u` | Jump to the agent that needs you | `agent next` | grofe's own |

## notifications

| Key | Does | Command | tmux |
|---|---|---|---|
| `y` | Show notifications | `choose-notification` | grofe's own |
| `~` | Show messages | `show-messages` | same |

## client

| Key | Does | Command | tmux |
|---|---|---|---|
| `C-z` | Suspend the current client | `suspend-client` | same |
| `D` | Choose and detach a client from a list | `choose-client -Z` | same |
| `DC` | Reset so the visible part of the window follows the cursor | `refresh-client -c` | same |
| `S-Down` | Move the visible part of the window down | `refresh-client -D 10` | same |
| `S-Left` | Move the visible part of the window left | `refresh-client -L 10` | same |
| `S-Right` | Move the visible part of the window right | `refresh-client -R 10` | same |
| `S-Up` | Move the visible part of the window up | `refresh-client -U 10` | same |
| `d` | Detach the current client | `detach-client` | same |
| `r` | Redraw the current client | `refresh-client` | same |

## copy

| Key | Does | Command | tmux |
|---|---|---|---|
| `#` | List all paste buffers | `list-buffers` | same |
| `-` | Delete the most recent paste buffer | `delete-buffer` | same |
| `=` | Choose a paste buffer from a list | `choose-buffer -Z` | same |
| `G` | Copy a match on screen by its label | `copy-mode -h` | grofe's own |
| `PPage` | Enter copy mode and scroll up | `copy-mode -u` | same |
| `[` | Enter copy mode | `copy-mode` | same |
| `]` | Paste the most recent paste buffer | `paste-buffer -p` | same |

## help

| Key | Does | Command | tmux |
|---|---|---|---|
| `/` | Describe key binding | `command-prompt -k -p key "list-keys -1N \"%%\""` | same |
| `?` | List key bindings | `list-keys -N` | same |
| `Space` | Show the key menu | `key-menu -T prefix` | tmux: Select next layout |

## prompt

| Key | Does | Command | tmux |
|---|---|---|---|
| `:` | Prompt for a command | `command-prompt` | same |

## settings

| Key | Does | Command | tmux |
|---|---|---|---|
| `C` | Open the settings menu | `settings` | tmux: Customize options |

## server

| Key | Does | Command | tmux |
|---|---|---|---|
| `Q` | Stop the server and every session | `confirm-before -p "kill-server: stop grofe and every session? (y/n)" kill-server` | grofe's own |

## other

| Key | Does | Command | tmux |
|---|---|---|---|
| `C-b` | Send the prefix key | `send-prefix` | same |
