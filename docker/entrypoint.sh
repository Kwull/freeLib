#!/bin/sh
# Container entrypoint: when started as root (the default), give the writable
# volumes to PUID:PGID (default 1000:1000) and drop privileges to that user
# before starting the server. Started with `--user`, it runs the command as is.
set -e

if [ "$(id -u)" = 0 ]; then
    PUID="${PUID:-1000}"
    PGID="${PGID:-1000}"
    case "$PUID$PGID" in *[!0-9]*) echo "PUID and PGID must be numeric" >&2; exit 1 ;; esac

    [ "$(id -g freelib)" = "$PGID" ] || groupmod -o -g "$PGID" freelib
    [ "$(id -u freelib)" = "$PUID" ] || usermod -o -u "$PUID" freelib

    for dir in "${FREELIB_DATA_DIR:-/data}" "${FREELIB_CACHE_DIR:-/cache}" "${FREELIB_EXPORT_DIR:-/export}"; do
        mkdir -p "$dir"
        # Only touch entries with the wrong owner, so repeated starts stay fast.
        if ! find "$dir" \( ! -user "$PUID" -o ! -group "$PGID" \) -exec chown -h "$PUID:$PGID" {} + 2>/dev/null; then
            echo "warning: could not change the owner of $dir to $PUID:$PGID (read-only or root-squashed mount?)" >&2
        fi
    done

    export HOME=/home/freelib
    exec setpriv --reuid="$PUID" --regid="$PGID" --init-groups -- "$@"
fi

exec "$@"
