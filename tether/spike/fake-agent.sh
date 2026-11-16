#!/usr/bin/env bash
# Run INSIDE a sandbox pane. Registers the pane as a "claude" agent (pane.report_agent), logs every
# received byte to $FAKE_LOG, goes working for $FAKE_WORK_SECS after each submitted prompt.
# Builds+runs a tiny C shim named `claude` so the foreground process name matches the agent
# (required for agent.prompt; see FINDINGS.md).
set -euo pipefail
: "${FAKE_LOG:?set FAKE_LOG}"
D="$(cd "$(dirname "$0")" && pwd)"
BIN="${TMPDIR:-/tmp}/tether-spike-bin"; mkdir -p "$BIN"
[ -x "$BIN/claude" ] && [ "$BIN/claude" -nt "$D/claude-shim.c" ] || cc -o "$BIN/claude" "$D/claude-shim.c"
export FAKE_PY="$(command -v python3)"
exec "$BIN/claude" "$D/fake_agent.py"
