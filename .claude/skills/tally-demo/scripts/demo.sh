#!/usr/bin/env bash
# The end-to-end demo for tally-demo: the fake API, the proxy and the fake program run from their
# binaries, then the proxy is stopped by SIGTERM, as Docker stops it. Linux only, since Windows
# can't stop the proxy cleanly from a script: from Git Bash it runs itself again under WSL.
# Always exits 0, so a failed step is reported instead of aborting the skill.

cd "$(dirname "$0")/../../../.." || { echo "demo.sh: can't find the repo root"; exit 0; }

case "$(uname -s)" in
  MINGW* | MSYS*)
    wsl.exe --cd "$(pwd -W)" bash .claude/skills/tally-demo/scripts/demo.sh \
      || echo "demo.sh: WSL can't run the demo (exit code $?)"
    exit 0
    ;;
esac

# A non-interactive shell doesn't read the profile that puts cargo on the PATH
if ! command -v cargo > /dev/null && [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi
# Under WSL, a target folder of its own, so the Linux and Windows builds don't fight
if grep -qi microsoft /proc/version 2> /dev/null; then
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/tally-demo-target}"
fi
target="${CARGO_TARGET_DIR:-target}/debug"

work="$(mktemp -d)"
api_pid=""
proxy_pid=""
trap 'kill $api_pid $proxy_pid 2> /dev/null; rm -rf "$work"' EXIT

# Each output read so far, then the end: called once a step fails, and at the end of the run
report() {
  if [ -f "$work/program.out" ]; then
    echo "== The program (exit code $program_exit)"
    cat "$work/program.out"
    echo
  fi
  if [ -f "$work/fake-api.out" ]; then
    echo "== What the fake API received"
    tr -d '\r' < "$work/fake-api.out"
    echo
  fi
  if [ -f "$work/proxy.err" ]; then
    echo "== The proxy (exit code ${proxy_exit:-none: still running})"
    cat "$work/proxy.out"
    if [ -s "$work/proxy.err" ]; then
      echo "stderr:"
      cat "$work/proxy.err"
    else
      echo "(nothing on stderr)"
    fi
    echo
  fi
  if [ -n "$log" ]; then
    echo "== This run's log: $log"
    cat "$log"
  fi
  exit 0
}

# Waits up to 10 s for the "listening on" line in file $1, while process $2 runs
wait_for_listening() {
  for _ in $(seq 100); do
    grep -q "^listening on" "$1" && return 0
    kill -0 "$2" 2> /dev/null || return 1
    sleep 0.1
  done
  return 1
}

echo "== Building"
if ! cargo build --quiet --locked -p fake-api -p proxy -p fake-program 2>&1; then
  echo "demo.sh: the build failed"
  exit 0
fi
echo "(done)"
echo

"$target/fake-api" 127.0.0.1:9001 > "$work/fake-api.out" 2>&1 &
api_pid=$!
if ! wait_for_listening "$work/fake-api.out" "$api_pid"; then
  echo "demo.sh: the fake API didn't start listening on 127.0.0.1:9001"
  report
fi

logs_before="$(ls demo/logs 2> /dev/null)"
"$target/proxy" demo/settings.toml demo/demo-secrets.toml > "$work/proxy.out" 2> "$work/proxy.err" &
proxy_pid=$!
if ! wait_for_listening "$work/proxy.out" "$proxy_pid"; then
  echo "demo.sh: the proxy didn't start listening"
  kill -KILL "$proxy_pid" 2> /dev/null
  wait "$proxy_pid"
  proxy_exit=$?
  report
fi
new_logs="$(comm -13 <(echo "$logs_before") <(ls demo/logs 2> /dev/null))"
log="demo/logs/$new_logs"

(
  set -a
  . demo/program.env
  set +a
  "$target/fake-program"
) > "$work/program.out" 2>&1
program_exit=$?

kill -TERM "$proxy_pid"
for _ in $(seq 100); do
  kill -0 "$proxy_pid" 2> /dev/null || break
  sleep 0.1
done
if kill -0 "$proxy_pid" 2> /dev/null; then
  echo "demo.sh: the proxy is still running 10 s after SIGTERM, so it was killed"
  kill -KILL "$proxy_pid"
fi
wait "$proxy_pid"
proxy_exit=$?
report
