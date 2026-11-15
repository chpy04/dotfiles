#!/usr/bin/env bash
# check-nvim.sh <pane-id>  -- nvim must already be running in that pane with --listen <tether_sock_path pane>
# Jumps: already-open file, another open file, brand-new file, line beyond EOF; verifies via --remote-expr.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
pane="$1"; sock="$(tether_sock_path "$pane")"; dir="${TETHER_SPIKE_DIR:?run via run-all.sh or set TETHER_SPIKE_DIR}"
real() { python3 -c 'import os,sys;print(os.path.realpath(sys.argv[1]))' "$1"; }
want() { # file line
  local got; got="$("$SPIKE_DIR/nvim-jump.sh" "$sock" "$1" "$2")"
  [ "$got" = "$(real "$1"):$2" ] || { echo "FAIL: jump $1:$2 returned $got"; exit 1; }
  # independent verification
  local v; v="$(nvim --server "$sock" --remote-expr 'expand("%:p").":".line(".")')"
  [ "$v" = "$(real "$1"):$2" ] || { echo "FAIL: state after jump $1:$2 is $v"; exit 1; }
}
mkdir -p "$dir"; [ -f "$dir/a.txt" ] || seq 1 300 | sed 's/^/line /' > "$dir/a.txt"; [ -f "$dir/b.txt" ] || seq 1 50 | sed 's/^/other /' > "$dir/b.txt"
rm -f "$dir/new.txt"
want "$dir/a.txt" 142    # may already be open
want "$dir/a.txt" 7      # same file again (already open and displayed)
want "$dir/b.txt" 20     # different file
want "$dir/a.txt" 250    # back to a buffer that is open but hidden
want "$dir/new.txt" 1    # file that does not exist yet
want "$dir/b.txt" 50     # last line
echo "PASS nvim"
