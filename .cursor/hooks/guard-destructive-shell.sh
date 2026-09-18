#!/usr/bin/env bash
# Ask/deny for clearly dangerous shell commands during agent runs.
# Fail-open on missing jq / bad JSON.
set -u

input=$(cat || true)

if ! command -v jq >/dev/null 2>&1; then
  echo '{"permission":"allow"}'
  exit 0
fi

if ! echo "$input" | jq -e . >/dev/null 2>&1; then
  echo '{"permission":"allow"}'
  exit 0
fi

cmd=$(echo "$input" | jq -r '.command // empty' 2>/dev/null || true)
if [[ -z "$cmd" ]]; then
  echo '{"permission":"allow"}'
  exit 0
fi

lc=$(printf '%s' "$cmd" | tr '[:upper:]' '[:lower:]')

deny() {
  local m="$1"
  jq -n --arg m "$m" '{permission:"deny", user_message:$m, agent_message:$m}'
  exit 0
}

ask() {
  local m="$1"
  jq -n --arg m "$m" '{permission:"ask", user_message:$m, agent_message:$m}'
  exit 0
}

if [[ "$lc" == *"git push --force"* || "$lc" == *"git push -f"* ]]; then
  deny "Blocked force-push. Never force-push shared branches."
fi
if [[ "$lc" == *"git reset --hard"* ]]; then
  deny "Blocked git reset --hard. Use safer recovery or ask the human."
fi
if [[ "$lc" == *"mkfs"* || "$lc" == *"diskutil erase"* ]]; then
  deny "Blocked destructive disk operation."
fi

# Bare root only: "rm -rf /" or "rm -fr /" as a path token (not /tmp, /Users, …)
if [[ "$lc" =~ (^|[[:space:];|&])rm[[:space:]]+(-[a-z0-9]*r[a-z0-9]*f[a-z0-9]*|-[a-z0-9]*f[a-z0-9]*r[a-z0-9]*)[[:space:]]+/($|[[:space:];|&]) ]]; then
  deny "Blocked recursive delete of filesystem root."
fi

# Ask — high impact but sometimes intentional in temp dirs
if [[ "$lc" == *"rm -rf"* || "$lc" == *"rm -fr"* ]]; then
  ask "Recursive delete requested. Confirm this path is intentional (prefer tempfile dirs)."
fi
if [[ "$lc" == *".raya/raya.db"* ]] && [[ "$lc" == *"rm "* || "$lc" == *"sqlite3"* ]]; then
  ask "Command touches .raya/raya.db. Confirm this is not a user's real project database."
fi

echo '{"permission":"allow"}'
exit 0
