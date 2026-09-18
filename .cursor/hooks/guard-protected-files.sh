#!/usr/bin/env bash
# Guard applied migrations (deny) and warn on product templates (allow + message).
# Fail-open: invalid JSON / missing jq → allow.
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

paths=$(echo "$input" | jq -r '
  [
    .tool_input.path?,
    .tool_input.file_path?,
    .tool_input.target_file?,
    .tool_input.path_to?,
    .input.path?,
    .input.file_path?,
    .input.target_file?,
    .path?,
    .file_path?,
    .target_file?
  ]
  | map(select(. != null and . != ""))
  | .[]
' 2>/dev/null || true)

normalize() {
  local p="$1"
  p="${p#./}"
  case "$p" in
    */Documents/GitHub/CodeAgent/*) p="${p##*/Documents/GitHub/CodeAgent/}" ;;
  esac
  if [[ -n "${CURSOR_PROJECT_DIR:-}" ]]; then
    local root="${CURSOR_PROJECT_DIR%/}/"
    if [[ "$p" == "$root"* ]]; then
      p="${p#"$root"}"
    fi
  fi
  # Absolute paths: keep trailing project-relative suffix when possible
  if [[ "$p" == /* ]]; then
    if [[ "$p" == */migrations/* ]]; then
      p="migrations/${p##*/migrations/}"
    elif [[ "$p" == */.raya/* ]]; then
      p=".raya/${p##*/.raya/}"
    elif [[ "$p" == */prompts/* ]]; then
      p="prompts/${p##*/prompts/}"
    fi
  fi
  printf '%s' "$p"
}

deny_msg=""
warn_msg=""

while IFS= read -r raw; do
  [[ -z "$raw" ]] && continue
  p="$(normalize "$raw")"

  if [[ "$p" == "migrations/0001_init.sql" || "$p" == "migrations/0002_index.sql" ]]; then
    deny_msg="Refusing to edit applied migration '$p'. Add migrations/000N_*.sql and wire it in raya-store instead."
    continue
  fi

  if [[ "$p" == .raya/rules/*.md ]]; then
    warn_msg="Note: '$p' is a product template shipped by raya init (include_str!). Prefer .cursor/rules for Cursor guidance."
  elif [[ "$p" == .raya/rules/* && "$p" == *.md ]]; then
    warn_msg="Note: '$p' is a product template shipped by raya init (include_str!). Prefer .cursor/rules for Cursor guidance."
  elif [[ "$p" == ".raya/config.toml.example" ]]; then
    warn_msg="Note: '$p' is a product template shipped by raya init. Changes affect every new project."
  elif [[ "$p" == "prompts/system.md" ]]; then
    warn_msg="Note: 'prompts/system.md' is compiled into the orchestrator via include_str!. Changing it alters AgentDecision prompting."
  fi
done <<< "$paths"

if [[ -n "$deny_msg" ]]; then
  jq -n --arg m "$deny_msg" '{permission:"deny", user_message:$m, agent_message:$m}'
  exit 0
fi

if [[ -n "$warn_msg" ]]; then
  jq -n --arg m "$warn_msg" '{permission:"allow", agent_message:$m}'
  exit 0
fi

echo '{"permission":"allow"}'
exit 0
