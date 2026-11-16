#!/usr/bin/env bash
# Run every gate: cargo test, clippy, the Neovim tests and the live sandbox tests of phases 1-3.
#   scripts/test-all.sh            all gates except the real-Claude end-to-end
#   scripts/test-all.sh --e2e      also run scripts/test-e2e.sh (a real model; costs a few cents)
# Exit status is non-zero if any gate failed; a summary is printed at the end.
# The live tests need Herdr running; they only touch sandbox workspaces (docs/PLAN.md safety rules).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/.." && pwd)"
E2E=0
for a in "$@"; do
  case "$a" in
    --e2e) E2E=1;;
    -h|--help) sed -n '2,6p' "$0"; exit 0;;
    *) echo "usage: $0 [--e2e]" >&2; exit 2;;
  esac
done
source ~/.cargo/env 2>/dev/null
cd "$ROOT" || exit 1

names=(); results=()
gate() { # <name> <command...>
  local name="$1"; shift
  echo; echo "=== $name"
  "$@"; local rc=$?
  names+=("$name"); results+=("$rc")
  [ "$rc" -eq 0 ] && echo "=== $name: ok" || echo "=== $name: FAILED (exit $rc)"
}

gate "cargo test"   cargo test --workspace -q
gate "cargo clippy" cargo clippy --workspace --all-targets -q -- -D warnings
gate "nvim tests"   nvim/tests/run.sh
gate "phase 1 live" scripts/test-phase1.sh
gate "phase 2 live" scripts/test-phase2.sh
gate "phase 3 live" scripts/test-phase3.sh
[ "$E2E" = 1 ] && gate "e2e (real Claude)" scripts/test-e2e.sh

echo; echo "=== summary"
rc=0
for i in "${!names[@]}"; do
  if [ "${results[$i]}" -eq 0 ]; then printf '  ok     %s\n' "${names[$i]}"; else printf '  FAILED %s\n' "${names[$i]}"; rc=1; fi
done
[ "$E2E" = 1 ] || echo "  (e2e skipped; pass --e2e to include it)"
exit $rc
