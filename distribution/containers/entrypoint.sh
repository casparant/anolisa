#!/bin/sh
# Tini reaps adopted descendants; exec preserves signals to the supervisor.
set -eu
runtime=${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR must name the container user private runtime directory}
case "$runtime" in
  /*) ;;
  *) echo 'XDG_RUNTIME_DIR must be absolute' >&2; exit 2 ;;
esac
if [ ! -d "$runtime" ] || [ -L "$runtime" ] \
    || [ "$(stat -c '%u:%a' -- "$runtime")" != "$(id -u):700" ]; then
  echo 'XDG_RUNTIME_DIR must be an existing user-owned directory with mode 0700' >&2
  exit 2
fi
exec anolisa "$@"
