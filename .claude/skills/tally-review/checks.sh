#!/usr/bin/env bash
# Facts for tally-review: what changed, then the formatter and the linter.
# Always exits 0, so a failed check is reported instead of aborting the skill.

cd "$(dirname "$0")/../../.." || { echo "checks.sh: can't find the repo root"; exit 0; }

run() {
  echo "\$ $*"
  "$@" 2>&1
  echo "(exit code $?)"
  echo
}

run git status --short --untracked-files=all
if git rev-parse --verify --quiet HEAD > /dev/null 2>&1; then
  run git diff HEAD
else
  echo "(no commit yet: every file listed above is new)"
  echo
fi
run cargo fmt --check
run cargo clippy --all-targets
exit 0
