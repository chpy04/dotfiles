#!/usr/bin/env bash
# Headless Neovim tests for the tether plugin (no Herdr needed: vim.system is stubbed).
#   nvim/tests/run.sh
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
rc=0

echo "== plugin tests =="
TETHER_TEST_FILE="$HERE/test_comment.lua" nvim --headless -u "$HERE/minimal_init.lua" </dev/null 2>&1
[ $? -eq 0 ] || rc=1

echo "== tether-nvim wrapper tests =="
bash "$HERE/test_wrapper.sh" || rc=1

[ $rc -eq 0 ] && echo "ALL NVIM TESTS PASSED" || echo "NVIM TESTS FAILED"
exit $rc
