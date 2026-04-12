#!/bin/bash
#
# vosh-notify.sh — Server-side helper script for Vosh push notifications.
#
# Install on your remote server. When a long-running command finishes,
# this script sends a push notification to your phone via Vosh's API.
#
# Usage:
#   vosh-notify <command>         # Run command and notify on completion
#   vosh-notify --watch <pid>     # Watch a PID and notify when it exits
#   vosh-notify --agent           # Watch Claude Code / AI agent for input prompts
#
# Setup:
#   1. Copy this script to your server: scp vosh-notify.sh server:~/bin/
#   2. chmod +x ~/bin/vosh-notify.sh
#   3. Set your device token: export VOSH_DEVICE_TOKEN="your-token-here"
#   4. (Optional) Set API endpoint: export VOSH_API_URL="https://api.vosh.app"
#

VOSH_API_URL="${VOSH_API_URL:-https://api.vosh.app}"
VOSH_DEVICE_TOKEN="${VOSH_DEVICE_TOKEN:-}"
SERVER_NAME="${HOSTNAME:-$(hostname)}"

notify() {
    local title="$1"
    local body="$2"
    local category="${3:-TASK_COMPLETE}"

    if [ -z "$VOSH_DEVICE_TOKEN" ]; then
        echo "[vosh] Warning: VOSH_DEVICE_TOKEN not set. Notification not sent."
        return 1
    fi

    curl -s -X POST "$VOSH_API_URL/v1/notify" \
        -H "Content-Type: application/json" \
        -d "{
            \"token\": \"$VOSH_DEVICE_TOKEN\",
            \"title\": \"$title\",
            \"body\": \"$body\",
            \"category\": \"$category\",
            \"server\": \"$SERVER_NAME\"
        }" > /dev/null 2>&1
}

# Run a command and notify on completion
run_and_notify() {
    local cmd="$*"
    local start_time=$(date +%s)

    echo "[vosh] Running: $cmd"
    eval "$cmd"
    local exit_code=$?
    local end_time=$(date +%s)
    local duration=$((end_time - start_time))

    if [ $exit_code -eq 0 ]; then
        notify "Task Complete" "$SERVER_NAME: \`$cmd\` finished in ${duration}s" "TASK_COMPLETE"
    else
        notify "Task Failed" "$SERVER_NAME: \`$cmd\` failed (exit $exit_code) after ${duration}s" "TASK_COMPLETE"
    fi

    return $exit_code
}

# Watch a PID and notify when it exits
watch_pid() {
    local pid="$1"

    if ! kill -0 "$pid" 2>/dev/null; then
        echo "[vosh] PID $pid is not running."
        return 1
    fi

    echo "[vosh] Watching PID $pid..."

    while kill -0 "$pid" 2>/dev/null; do
        sleep 5
    done

    wait "$pid" 2>/dev/null
    local exit_code=$?

    notify "Process Exited" "$SERVER_NAME: PID $pid exited with code $exit_code" "TASK_COMPLETE"
}

# Watch for AI agent input prompts (Claude Code, etc.)
watch_agent() {
    echo "[vosh] Watching for AI agent input prompts..."
    echo "[vosh] Monitoring terminal for common prompt patterns..."

    # Watch tmux pane output for common AI agent prompt patterns
    local patterns="(y/n)|(\[Y/n\])|(\[yes/no\])|(Enter to continue)|(Press any key)|(waiting for input)|(Type your)"

    if command -v tmux &> /dev/null && tmux list-sessions &> /dev/null; then
        # Monitor tmux pane
        tmux pipe-pane -o "cat >> /tmp/vosh_agent_watch.log"

        while true; do
            if [ -f /tmp/vosh_agent_watch.log ]; then
                if grep -qE "$patterns" /tmp/vosh_agent_watch.log 2>/dev/null; then
                    local prompt=$(grep -oE ".{0,50}($patterns).{0,50}" /tmp/vosh_agent_watch.log | tail -1)
                    notify "Agent Needs Input" "$SERVER_NAME: $prompt" "AGENT_NEEDS_INPUT"
                    > /tmp/vosh_agent_watch.log  # Clear log
                fi
            fi
            sleep 3
        done
    else
        echo "[vosh] tmux not available. Watching stdout instead."
        # Fallback: pipe stdin
        while IFS= read -r line; do
            if echo "$line" | grep -qE "$patterns"; then
                notify "Agent Needs Input" "$SERVER_NAME: $line" "AGENT_NEEDS_INPUT"
            fi
        done
    fi
}

# Main
case "${1:-}" in
    --watch)
        watch_pid "$2"
        ;;
    --agent)
        watch_agent
        ;;
    --help|-h)
        echo "Usage:"
        echo "  vosh-notify <command>       Run command and notify on completion"
        echo "  vosh-notify --watch <pid>   Watch PID and notify on exit"
        echo "  vosh-notify --agent         Watch for AI agent input prompts"
        echo ""
        echo "Environment:"
        echo "  VOSH_DEVICE_TOKEN   Your device push token (required)"
        echo "  VOSH_API_URL        API endpoint (default: https://api.vosh.app)"
        ;;
    "")
        echo "[vosh] Error: No command specified. Use --help for usage."
        exit 1
        ;;
    *)
        run_and_notify "$@"
        ;;
esac
