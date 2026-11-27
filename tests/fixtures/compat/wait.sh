#!/bin/sh
# Waits, for at most 10 s (100 tries 0.1 s apart), until:
#   wait.sh exists F...        every file F exists
#   wait.sh file F...          every file F exists and is not empty
#   wait.sh lines F N          file F has at least N lines
#   wait.sh line F TEXT        file F has TEXT as a whole line
#   wait.sh shows T LINE [N]   pane T (its history too) has LINE as a whole
#                              row, N times
#   wait.sh has T TEXT         pane T (its history too) has TEXT anywhere
#   wait.sh noecho T...        each pane T's tty has echo off (its stty ran)
#   wait.sh format T FMT VAL   `display -p -t T FMT` prints VAL (in a
#                              run-shell, write FMT's # as ##)
#   wait.sh gone T             target T no longer exists
#   wait.sh clients N          the server has N clients
# A pane's program, or a pipe's command, runs after the command that started
# it returns, in tmux as in grofe, so a scenario waits on what it does with
# this, never a fixed sleep. On a timeout it says so on stderr, so the
# golden (and a grofe run that differs from it) shows the miss.
what=$1
shift
check() {
  case $what in
  exists)
    for f; do [ -e "$f" ] || return 1; done ;;
  file)
    for f; do [ -s "$f" ] || return 1; done ;;
  lines)
    [ -f "$1" ] && [ "$(wc -l < "$1")" -ge "$2" ] ;;
  line)
    grep -qxF -- "$2" "$1" 2>/dev/null ;;
  shows)
    [ "$(tmux capture-pane -p -S - -t "$1" | grep -cxF -- "$2")" -ge "${3:-1}" ] ;;
  has)
    tmux capture-pane -p -S - -t "$1" | grep -qF -- "$2" ;;
  noecho)
    for t; do
      stty -a < "$(tmux display -p -t "$t" '#{pane_tty}')" | grep -Eq '(^|[ ;])-echo([ ;]|$)' || return 1
    done ;;
  format)
    [ "$(tmux display -p -t "$1" "$2" 2>/dev/null)" = "$3" ] ;;
  seen)
    seen=$(tmux display -p -t "$1" "$2" 2>/dev/null)
    [ "$seen" = "$3" ] ;;
  clients)
    [ "$(tmux list-clients | wc -l)" -eq "$1" ] ;;
  gone)
    ! tmux list-panes -t "$1" >/dev/null 2>&1 ;;
  *)
    echo "wait.sh: unknown condition $what" >&2
    exit 1 ;;
  esac
}
i=0
until check "$@"; do
  i=$((i + 1))
  if [ "$i" -ge 100 ]; then
    echo "wait.sh: timed out: $what $*" >&2
    break
  fi
  sleep 0.1
done
# seen: print the value the wait read last, so one read both waits for a
# value that does not stay (mode 2026's flag) and records it.
[ "$what" != seen ] || echo "$seen"
exit 0
