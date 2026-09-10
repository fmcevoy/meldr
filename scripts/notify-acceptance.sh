#!/usr/bin/env bash
# Acceptance check for meldr's Claude notification routing.
#
# Builds a realistic layout on an isolated tmux server — one window with three
# agent panes sharing a worktree directory, plus a second window that is the
# *active* one — then fires hook events from each interesting position and prints
# what actually lit up next to what should have.
#
# The active-window part matters: the original bug lit whichever tab you happened
# to be looking at, so a check performed while looking at the target window would
# have passed throughout.
#
# Usage: ./scripts/notify-acceptance.sh [path-to-meldr]
# Exits non-zero on any mismatch.

set -uo pipefail

MELDR="${1:-$(cd "$(dirname "$0")/.." && pwd)/target/debug/meldr}"
if [[ ! -x "$MELDR" ]]; then
    echo "error: no meldr binary at $MELDR (run: cargo build)" >&2
    exit 2
fi

command -v tmux >/dev/null || { echo "error: tmux not on PATH" >&2; exit 2; }

# Isolated server. TMUX_TMPDIR must exist — tmux silently falls back to /tmp when
# it does not, which would aim this script at the user's own session.
# This script is often run from inside a Claude session, and a tmux server hands
# its own environment to every pane it creates. Clear the variables that would
# otherwise reach the panes and change which resolver tier runs.
unset CLAUDE_CODE_CHILD_SESSION MELDR_TMUX_PANE MELDR_TMUX_WINDOW_ID MELDR_AGENT_SESSION

ROOT="$(mktemp -d)"
export TMUX_TMPDIR="$ROOT/tmux"
mkdir -p "$TMUX_TMPDIR"
HOME_DIR="$ROOT/home"
WORK="$ROOT/worktree"
OTHER="$ROOT/elsewhere"
mkdir -p "$HOME_DIR" "$WORK" "$OTHER"

cleanup() {
    tmux kill-server 2>/dev/null
    rm -rf "$ROOT"
}
trap cleanup EXIT

# Never inherit the caller's Claude/meldr variables: this script is often run from
# inside a Claude session, and those would change which resolver tier runs.
run_meldr() {
    env -u TMUX -u TMUX_PANE -u CLAUDE_CODE_CHILD_SESSION \
        -u MELDR_TMUX_PANE -u MELDR_TMUX_WINDOW_ID -u MELDR_AGENT_SESSION \
        HOME="$HOME_DIR" MELDR_CC_TIMEOUT=300 "$@"
}

# ── layout ────────────────────────────────────────────────────────────────────
read -r WIN_A PANE_1 <<<"$(tmux new-session -d -s acc -x 200 -y 50 -c "$WORK" \
    -P -F '#{window_id} #{pane_id}')"
PANE_2=$(tmux split-window -t "$PANE_1" -d -c "$WORK" -P -F '#{pane_id}')
PANE_3=$(tmux split-window -t "$PANE_1" -d -c "$WORK" -P -F '#{pane_id}')
read -r WIN_B _ <<<"$(tmux new-window -t acc -c "$OTHER" -P -F '#{window_id} #{pane_id}')"
tmux select-window -t "$WIN_B"   # the window in front of you is NOT the target

ACTIVE=$(tmux display-message -p '#{window_id}')

echo "server        : $(tmux display-message -p '#{socket_path}')"
echo "worktree win  : $WIN_A  panes $PANE_1 $PANE_2 $PANE_3  (cwd $WORK)"
echo "other window  : $WIN_B  (ACTIVE: $ACTIVE)"
echo

FAILURES=0

pane_status() { tmux show-options -pqv -t "$1" @cc_pane_status 2>/dev/null; }
win_status()  { tmux show-options -wqv -t "$1" @cc_status 2>/dev/null; }

reset_all() {
    for p in $(tmux list-panes -a -F '#{pane_id}'); do
        tmux set-option -pu -t "$p" @cc_pane_status 2>/dev/null
        tmux set-option -pu -t "$p" @cc_pane_until 2>/dev/null
    done
    for w in $(tmux list-windows -a -F '#{window_id}'); do
        tmux set-option -wu -t "$w" @cc_status 2>/dev/null
        tmux set-option -wu -t "$w" @cc_until 2>/dev/null
    done
}

# Fire a hook from a shell nested two `sh -c` levels inside a pane, the way Claude
# invokes one, and wait for it to finish.
fire_in_pane() {
    local pane="$1" event="$2" payload="$3"
    local done="$ROOT/done.$$" log="$ROOT/log.$$"
    rm -f "$done" "$log"
    tmux send-keys -t "$pane" \
        "printf %s '$payload' | env -u TMUX_PANE -u CLAUDE_CODE_CHILD_SESSION -u MELDR_TMUX_PANE -u MELDR_TMUX_WINDOW_ID HOME='$HOME_DIR' MELDR_CC_TIMEOUT=300 sh -c 'sh -c \"$MELDR claude-hook $event\"' >'$log' 2>&1; touch '$done'" Enter
    for _ in $(seq 1 200); do [[ -f "$done" ]] && break; sleep 0.1; done
}

stop_payload() {
    printf '{"hook_event_name":"Stop","session_id":"acc","cwd":"%s","last_assistant_message":"%s"}' "$1" "$2"
}

printf '%-46s %-10s %-12s %-12s %s\n' SCENARIO EXPECT "PANE" "WINDOW" RESULT
printf '%s\n' "------------------------------------------------------------------------------------------"

check() {
    local scenario="$1" expect="$2" got_pane="$3" got_win="$4" ok="$5"
    local verdict="ok"
    if [[ "$ok" != "1" ]]; then verdict="FAIL"; FAILURES=$((FAILURES + 1)); fi
    printf '%-46s %-10s %-12s %-12s %s\n' \
        "$scenario" "$expect" "${got_pane:-–}" "${got_win:-–}" "$verdict"
}

# 1–3. Each agent pane must light itself and its own tab, never the active one.
i=1
for pane in "$PANE_1" "$PANE_2" "$PANE_3"; do
    reset_all
    fire_in_pane "$pane" stop "$(stop_payload "$WORK" 'All done.')"
    ps_=$(pane_status "$pane"); ws=$(win_status "$WIN_A"); other=$(win_status "$WIN_B")
    siblings_dark=1
    for s in "$PANE_1" "$PANE_2" "$PANE_3"; do
        [[ "$s" == "$pane" ]] && continue
        [[ -n "$(pane_status "$s")" ]] && siblings_dark=0
    done
    ok=$([[ "$ps_" == "done" && "$ws" == "done" && -z "$other" && "$siblings_dark" == 1 ]] && echo 1 || echo 0)
    check "agent pane $i ($pane) Stop" "done" "$ps_" "$ws" "$ok"
    i=$((i + 1))
done

# 4. A question means the agent is waiting on you.
reset_all
fire_in_pane "$PANE_2" stop "$(stop_payload "$WORK" 'Which branch?')"
ps_=$(pane_status "$PANE_2"); ws=$(win_status "$WIN_A")
ok=$([[ "$ps_" == "waiting" && "$ws" == "waiting" ]] && echo 1 || echo 0)
check "question in $PANE_2" "waiting" "$ps_" "$ws" "$ok"

# 5. waiting outranks done across panes in one window.
reset_all
fire_in_pane "$PANE_1" stop "$(stop_payload "$WORK" 'Done.')"
fire_in_pane "$PANE_3" stop "$(stop_payload "$WORK" 'Proceed?')"
ws=$(win_status "$WIN_A")
ok=$([[ "$ws" == "waiting" ]] && echo 1 || echo 0)
check "tab shows the most urgent pane" "waiting" "$(pane_status "$PANE_1")/$(pane_status "$PANE_3")" "$ws" "$ok"

# 6. A background job: no pane ancestry, no $TMUX — window only, marked as such.
reset_all
echo "$(stop_payload "$WORK" 'Done.')" | (cd "$WORK" && run_meldr "$MELDR" claude-hook stop) >/dev/null 2>&1
ws=$(win_status "$WIN_A"); other=$(win_status "$WIN_B")
panes_dark=1
for s in "$PANE_1" "$PANE_2" "$PANE_3"; do [[ -n "$(pane_status "$s")" ]] && panes_dark=0; done
ok=$([[ "$ws" == "bg-done" && -z "$other" && "$panes_dark" == 1 ]] && echo 1 || echo 0)
check "detached background job" "bg-done" "(none)" "$ws" "$ok"

# 7. A stale MELDR_TMUX_PANE naming another window's pane must be ignored.
reset_all
DECOY=$(tmux list-panes -t "$WIN_B" -F '#{pane_id}' | head -1)
tmux send-keys -t "$PANE_1" \
    "printf %s '$(stop_payload "$WORK" 'Done.')' | env -u CLAUDE_CODE_CHILD_SESSION MELDR_TMUX_PANE='$DECOY' MELDR_TMUX_WINDOW_ID='$WIN_B' HOME='$HOME_DIR' MELDR_CC_TIMEOUT=300 sh -c 'sh -c \"$MELDR claude-hook stop\"' >/dev/null 2>&1; touch '$ROOT/d7'" Enter
for _ in $(seq 1 200); do [[ -f "$ROOT/d7" ]] && break; sleep 0.1; done
ok=$([[ "$(pane_status "$PANE_1")" == "done" && -z "$(pane_status "$DECOY")" && -z "$(win_status "$WIN_B")" ]] && echo 1 || echo 0)
check "poisoned MELDR_TMUX_PANE ignored" "done" "$(pane_status "$PANE_1")" "$(win_status "$WIN_A")" "$ok"

# 8. Routine notifications must not claim your attention.
reset_all
fire_in_pane "$PANE_1" notify \
    '{"hook_event_name":"Notification","session_id":"acc","cwd":"'"$WORK"'","notification_type":"auth_success"}'
ok=$([[ -z "$(win_status "$WIN_A")" ]] && echo 1 || echo 0)
check "auth_success does not flash" "(none)" "$(pane_status "$PANE_1")" "$(win_status "$WIN_A")" "$ok"

# 9. A permission prompt does.
reset_all
fire_in_pane "$PANE_1" notify \
    '{"hook_event_name":"Notification","session_id":"acc","cwd":"'"$WORK"'","notification_type":"permission_prompt"}'
ok=$([[ "$(win_status "$WIN_A")" == "waiting" ]] && echo 1 || echo 0)
check "permission_prompt flashes" "waiting" "$(pane_status "$PANE_1")" "$(win_status "$WIN_A")" "$ok"

# 10. Expiry, with two overlapping lifetimes in flight at once.
reset_all
tmux send-keys -t "$PANE_1" \
    "printf %s '$(stop_payload "$WORK" 'Done.')' | env HOME='$HOME_DIR' MELDR_CC_TIMEOUT=2 sh -c 'sh -c \"$MELDR claude-hook stop\"' >/dev/null 2>&1" Enter
sleep 0.5
tmux send-keys -t "$PANE_2" \
    "printf %s '$(stop_payload "$WORK" 'Done.')' | env HOME='$HOME_DIR' MELDR_CC_TIMEOUT=4 sh -c 'sh -c \"$MELDR claude-hook stop\"' >/dev/null 2>&1" Enter
sleep 9
ok=$([[ -z "$(pane_status "$PANE_1")" && -z "$(pane_status "$PANE_2")" && -z "$(win_status "$WIN_A")" ]] && echo 1 || echo 0)
check "overlapping flashes both expire" "(none)" "$(pane_status "$PANE_1")/$(pane_status "$PANE_2")" "$(win_status "$WIN_A")" "$ok"

# 11. The self-test agrees with tmux about where it is.
reset_all
SELFTEST=$(tmux send-keys -t "$PANE_3" \
    "env -u CLAUDE_CODE_CHILD_SESSION HOME='$HOME_DIR' sh -c 'sh -c \"$MELDR claude-hook selftest\"' >'$ROOT/st' 2>&1; touch '$ROOT/d11'" Enter; \
    for _ in $(seq 1 200); do [[ -f "$ROOT/d11" ]] && break; sleep 0.1; done; cat "$ROOT/st" 2>/dev/null)
got_pane=$(printf '%s' "$SELFTEST" | sed -n 's/.*"pane":"\([^"]*\)".*/\1/p')
got_win=$(printf '%s' "$SELFTEST" | sed -n 's/.*"window":"\([^"]*\)".*/\1/p')
ok=$([[ "$got_pane" == "$PANE_3" && "$got_win" == "$WIN_A" ]] && echo 1 || echo 0)
check "selftest in $PANE_3" "$PANE_3" "$got_pane" "$got_win" "$ok"

echo
if [[ "$FAILURES" -eq 0 ]]; then
    echo "all scenarios correct"
else
    echo "$FAILURES scenario(s) FAILED"
fi
exit $((FAILURES > 0))
