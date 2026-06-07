#!/usr/bin/env bash
# meldr-agent-notify.sh — managed by meldr; do not edit by hand.
# Installed to ~/.local/share/meldr/ by `meldr install-hooks`.
# Wired into ~/.claude/settings.json Stop + Notification hooks.
#
# On each event:
#   - Plays a distinct sound (afplay, backgrounded so the hook doesn't block)
#   - Sets @cc_status on the tmux window so the tab flashes
#   - Sets @cc_pane_status on the agent pane for the border indicator
#   - Writes ~/.cache/claude-agents/<session_id>.json for dashboard tools

# MELDR_AGENT_NOTIFY_DRY_RUN=1 makes the script print tmux commands instead of
# running them. Used in integration tests that run without a live tmux server.
DRY_RUN="${MELDR_AGENT_NOTIFY_DRY_RUN:-0}"

EVENT="${1:-stop}"
STATE_DIR="$HOME/.cache/claude-agents"
mkdir -p "$STATE_DIR"

# Read hook JSON from stdin (1s timeout so a broken caller can't hang)
HOOK_DATA=$(timeout 1 cat 2>/dev/null || true)

# Skip subagent events only. A tighter check than has("agent_id"): skip only
# when agent_id is non-empty AND the event name explicitly identifies a
# subagent (e.g. SubagentStop). Main-agent Stop events still flash.
if printf '%s' "$HOOK_DATA" | jq -e '(.agent_id // "") != "" and ((.hook_event_name // "") | test("Subagent"))' >/dev/null 2>&1; then
  exit 0
fi

SESSION_ID=$(printf '%s' "$HOOK_DATA" | jq -r '.session_id // ""' 2>/dev/null)
CWD=$(printf '%s' "$HOOK_DATA" | jq -r '.cwd // ""' 2>/dev/null)

# Resolve pane context. Priority:
#   1. MELDR_TMUX_PANE — injected by meldr at agent spawn time (most reliable)
#   2. TMUX_PANE       — inherited from shell when env propagation works
#   3. Sidecar file    — written by meldr at spawn, keyed by MELDR_AGENT_SESSION
PANE_ID=""
WINDOW_ID=""
WINDOW_NAME=""
if [ -n "${MELDR_TMUX_PANE:-}" ]; then
  PANE_ID="$MELDR_TMUX_PANE"
  WINDOW_ID="${MELDR_TMUX_WINDOW_ID:-}"
  WINDOW_NAME=$(tmux display-message -t "$PANE_ID" -p '#{window_name}' 2>/dev/null || true)
elif [ -n "${TMUX:-}" ] && [ -n "${TMUX_PANE:-}" ]; then
  PANE_ID=$(tmux display-message -t "$TMUX_PANE" -p '#{pane_id}' 2>/dev/null || true)
  WINDOW_ID=$(tmux display-message -t "$TMUX_PANE" -p '#{window_id}' 2>/dev/null || true)
  WINDOW_NAME=$(tmux display-message -t "$TMUX_PANE" -p '#{window_name}' 2>/dev/null || true)
elif [ -n "${MELDR_AGENT_SESSION:-}" ]; then
  SIDECAR="$STATE_DIR/${MELDR_AGENT_SESSION}.parent_pane"
  PANE_ID=$(cat "$SIDECAR" 2>/dev/null || true)
  [ -n "$PANE_ID" ] && WINDOW_ID=$(tmux display-message -t "$PANE_ID" -p '#{window_id}' 2>/dev/null || true)
  [ -n "$PANE_ID" ] && WINDOW_NAME=$(tmux display-message -t "$PANE_ID" -p '#{window_name}' 2>/dev/null || true)
fi

# Map event to status and sound
STATUS="$EVENT"
case "$EVENT" in
  stop|Stop)
    STATUS="done"
    afplay /System/Library/Sounds/Glass.aiff 2>/dev/null &
    ;;
  notify)
    STATUS="waiting"
    afplay /System/Library/Sounds/Funk.aiff 2>/dev/null &
    ;;
esac

# Atomic state-file write so concurrent reads never see a partial file
if [ -n "$SESSION_ID" ]; then
  TMP=$(mktemp "$STATE_DIR/.${SESSION_ID}.XXXXXX")
  printf '{"status":"%s","ts":%s,"cwd":"%s","pane":"%s","window":"%s","window_name":"%s"}\n' \
    "$STATUS" "$(date +%s)" "$CWD" "$PANE_ID" "$WINDOW_ID" "$WINDOW_NAME" \
    > "$TMP" && mv -f "$TMP" "$STATE_DIR/${SESSION_ID}.json" || rm -f "$TMP"
fi

# Flash the window tab and pane border.
# Generation guard: each flash stores a unique token in @cc_status_gen so that
# one pane's 120s clear-timer doesn't wipe a later flash from another pane.
if [ -n "$WINDOW_ID" ]; then
  GEN="$(date +%s%N)-$$"
  TIMEOUT="${MELDR_CC_TIMEOUT:-120}"

  if [ "$DRY_RUN" = "1" ]; then
    echo "tmux set-option -w -t $WINDOW_ID @cc_status $STATUS"
    echo "tmux set-option -w -t $WINDOW_ID @cc_status_gen $GEN"
    [ -n "$PANE_ID" ] && echo "tmux set-option -p -t $PANE_ID @cc_pane_status $STATUS"
    echo "tmux run-shell -b (clear timer after ${TIMEOUT}s)"
  else
    tmux set-option -w -t "$WINDOW_ID" @cc_status "$STATUS" 2>/dev/null || true
    tmux set-option -w -t "$WINDOW_ID" @cc_status_gen "$GEN" 2>/dev/null || true
    [ -n "$PANE_ID" ] && tmux set-option -p -t "$PANE_ID" @cc_pane_status "$STATUS" 2>/dev/null || true
    tmux run-shell -b "sleep $TIMEOUT; \
      CUR=\$(tmux show-options -wqv -t '$WINDOW_ID' @cc_status_gen 2>/dev/null); \
      [ \"\$CUR\" = '$GEN' ] && tmux set-option -wu -t '$WINDOW_ID' @cc_status 2>/dev/null; \
      tmux set-option -wu -t '$WINDOW_ID' @cc_status_gen 2>/dev/null; \
      [ -n '$PANE_ID' ] && tmux set-option -pu -t '$PANE_ID' @cc_pane_status 2>/dev/null" \
      2>/dev/null || true
  fi
fi
