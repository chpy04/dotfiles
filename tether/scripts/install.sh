#!/usr/bin/env bash
# Install tether for daily use (symlinks, so a rebuild or `git pull` takes effect at once):
#   target/release/tether  -> ~/.local/bin/tether
#   bin/tether-nvim        -> ~/.local/bin/tether-nvim
#   skill/                 -> ~/.claude/skills/tether
#
#   scripts/install.sh [--dry-run] [--no-build]
#   scripts/install.sh --uninstall [--dry-run]
#
# Idempotent. Never overwrites a real file or directory (only symlinks it made or can safely
# re-point); if anything is in the way it changes nothing and says what. --dry-run prints every
# action without doing any of them (no build, no links).
# Overrides (mostly for testing): TETHER_INSTALL_BIN_DIR, TETHER_INSTALL_SKILLS_DIR.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
BIN_DIR="${TETHER_INSTALL_BIN_DIR:-$HOME/.local/bin}"
SKILLS_DIR="${TETHER_INSTALL_SKILLS_DIR:-$HOME/.claude/skills}"
dry=0; uninstall=0; build=1
for a in "$@"; do
  case "$a" in
    --dry-run) dry=1 ;;
    --uninstall) uninstall=1 ;;
    --no-build) build=0 ;;
    -h|--help) sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $a (see --help)" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*"; }
act() { if [ "$dry" = 1 ]; then say "[dry-run] $*"; else say "$*"; fi; }

# "src dest" pairs
SRC=("$ROOT/target/release/tether" "$ROOT/bin/tether-nvim" "$ROOT/skill")
DST=("$BIN_DIR/tether" "$BIN_DIR/tether-nvim" "$SKILLS_DIR/tether")

problems=0
refuse() { say "refusing: $1" >&2; problems=$((problems + 1)); }

# ---- preflight: change nothing if anything is in the way ------------------------------------------
for i in "${!DST[@]}"; do
  d="${DST[$i]}"; s="${SRC[$i]}"
  if [ -L "$d" ]; then
    if [ "$uninstall" = 1 ]; then
      case "$(readlink "$d")" in "$ROOT"/*) ;; *) refuse "$d is a symlink to $(readlink "$d"), not to this checkout; leaving it" ;; esac
    fi
  elif [ -e "$d" ]; then
    refuse "$d exists and is not a symlink; move it away first (tether never overwrites real files)"
  fi
done
if [ "$problems" -gt 0 ]; then
  say "nothing was changed." >&2
  exit 1
fi

# ---- uninstall ----------------------------------------------------------------------------------------
if [ "$uninstall" = 1 ]; then
  for d in "${DST[@]}"; do
    if [ -L "$d" ]; then
      act "remove symlink $d"
      [ "$dry" = 1 ] || rm "$d"
    else
      say "skip $d (not installed)"
    fi
  done
  say "done. (Your tether config and state files are untouched.)"
  exit 0
fi

# ---- build -------------------------------------------------------------------------------------------------
if [ "$build" = 1 ]; then
  act "cargo build --release (in $ROOT)"
  if [ "$dry" = 0 ]; then
    # shellcheck disable=SC1090
    [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
    (cd "$ROOT" && cargo build --release -p tether)
  fi
fi
if [ "$dry" = 0 ] && [ ! -x "$ROOT/target/release/tether" ]; then
  say "error: $ROOT/target/release/tether does not exist (build failed or --no-build without a build)" >&2
  exit 1
fi

# ---- link ---------------------------------------------------------------------------------------------------
for i in "${!DST[@]}"; do
  d="${DST[$i]}"; s="${SRC[$i]}"
  if [ -L "$d" ] && [ "$(readlink "$d")" = "$s" ]; then
    say "ok   $d -> $s (already linked)"
    continue
  fi
  if [ -L "$d" ]; then
    act "re-point symlink $d (was $(readlink "$d")) -> $s"
    [ "$dry" = 1 ] || ln -sfn "$s" "$d"
  else
    act "link $d -> $s"
    if [ "$dry" = 0 ]; then
      mkdir -p "$(dirname "$d")"
      ln -s "$s" "$d"
    else
      [ -d "$(dirname "$d")" ] || say "[dry-run]   (would create $(dirname "$d"))"
    fi
  fi
done

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "note: $BIN_DIR is not on your PATH; add it so \`tether\` and \`tether-nvim\` are found" ;;
esac
say "done. Start a new Claude Code session to pick up the skill; see docs/INSTALL.md for the Neovim plugin."
