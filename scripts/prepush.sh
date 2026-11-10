#!/bin/sh
# Checks the commits about to go to origin/main for secrets and personal data,
# and fails if it finds any: gitleaks for keys and tokens, and added lines
# with personal paths or the app's pre-rename name. (Fixtures stay synthetic,
# never from a real state.json, board or transcript; no script can tell.)
# Run it before every push:
# `sh scripts/prepush.sh && git push origin main`.
set -u
range="${1:-origin/main..HEAD}"
status=0

if ! command -v gitleaks >/dev/null 2>&1; then
  echo "prepush: gitleaks isn't installed (brew install gitleaks)." >&2
  exit 1
fi

if ! gitleaks git --redact --no-banner --log-opts="$range" . >&2; then
  echo "prepush: gitleaks found possible secrets." >&2
  status=1
fi

# Personal paths and the pre-rename app name, in added lines only.
# Made-up placeholders such as /Users/example and /Users/<name> are fine, and so
# is the one constant that names the old app's data folder, as store.ts does.
found=$(git log -p --no-color "$range" \
  | grep -niE '^\+.*(/Users/[a-z]|/private/tmp/claude-[0-9]|claude.?nodes)' \
  | grep -vE '/Users/(example|<)' \
  | grep -vE '^[0-9]+:\+const OLD_APP_NAME: &str = ' \
  | head -20)
if [ -n "$found" ]; then
  echo "prepush: personal paths or the old app name in added lines:" >&2
  echo "$found" >&2
  status=1
fi

[ "$status" -eq 0 ] && echo "prepush: clean ($(git rev-list --count "$range") commits)."
exit "$status"
