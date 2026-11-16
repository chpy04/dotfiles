# sourced helpers
SPIKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rpc() { python3 "$SPIKE_DIR/herdr_rpc.py" "$@"; }
# runtime dir for tether sockets: $XDG_RUNTIME_DIR, else $TMPDIR (trailing slash stripped), else /tmp
tether_sock_dir() { local b="${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}"; echo "${b%/}/tether"; }
# ':' in a pane id would be parsed by nvim as host:port, so map it to '-'
tether_sock_path() { echo "$(tether_sock_dir)/${1//:/-}.sock"; }
