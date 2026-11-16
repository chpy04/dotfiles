#!/usr/bin/env bash
# Renders the diagram fixtures to docs/screens/: *.txt (plain Unicode glyphs) and *.ans (truecolor +
# Nerd Font icons; `cat` it in a truecolor terminal to see the real look).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source ~/.cargo/env 2>/dev/null || true
(cd "$ROOT" && cargo build -q -p tether)
T="$ROOT/target/debug/tether"
F="$ROOT/crates/tether-core/tests/fixtures/diagrams"
OUT="$ROOT/docs/screens"; mkdir -p "$OUT"
export TETHER_CONFIG=/nonexistent/tether.toml
render() { # name fixture focus [expand]
  local name="$1" fx="$2" focus="$3" expand="${4:-}"
  local args=(diagram render "$F/$fx.json" --focus "$focus" --no-resolve --width 100)
  [ -n "$expand" ] && args+=(--expand "$expand")
  "$T" "${args[@]}" --no-icons > "$OUT/$name.txt"
  "$T" "${args[@]}" --ansi > "$OUT/$name.ans"
  echo "wrote $OUT/$name.txt and .ans"
}
render prd_flowchart prd_flowchart check
render prd_flowchart_expanded prd_flowchart check check
render session_refresh_30 session_refresh_30 check
render login_sequence login_sequence m2 m2
