#!/usr/bin/env bash
# Wire the tuning file into the freshly initialised cluster.
#
# The official entrypoint runs initdb, starts a temporary server, executes these
# scripts, stops it, then execs the real postgres. Appending an include here
# therefore takes effect on the final start, which is why this is a config edit
# rather than a set of -c flags on CMD.
set -euo pipefail

if ! grep -q "parallax.conf" "$PGDATA/postgresql.conf"; then
  printf "\n# Parallax tuning\ninclude '/etc/postgresql/parallax.conf'\n" \
    >> "$PGDATA/postgresql.conf"
fi
