#!/usr/bin/env bash
# Playwright `webServer.command` for `pnpm test:real`: builds a small synthetic library with
# the server's own `gen-inpx --with-files` tool, then execs the release server over it, with
# `FREELIB_WEB_DIR` pointing at the already-built `web/dist`. `exec` at the end replaces this
# shell with the server process, so Playwright's SIGTERM at the end of the run reaches it
# directly instead of leaving it orphaned.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEB_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$WEB_DIR/.." && pwd)"
SERVER_DIR="$ROOT_DIR/server"

if [ ! -f "$WEB_DIR/dist/index.html" ]; then
  echo "[real-server] web/dist is missing an index.html; run 'pnpm build' first" >&2
  exit 1
fi

TMP_DIR="${FREELIB_REAL_E2E_DIR:-}"
if [ -z "$TMP_DIR" ]; then
  TMP_DIR="$(mktemp -d -t freelib-real-e2e.XXXXXX)"
fi
DATA_DIR="$TMP_DIR/data"
CACHE_DIR="$TMP_DIR/cache"
BOOKS_DIR="$TMP_DIR/books"
EXPORT_DIR="$TMP_DIR/export"
mkdir -p "$DATA_DIR" "$CACHE_DIR" "$BOOKS_DIR" "$EXPORT_DIR"

BOOKS="${FREELIB_REAL_E2E_BOOKS:-20000}"
INPX="$BOOKS_DIR/lib.inpx"

if [ ! -f "$INPX" ]; then
  echo "[real-server] generating a synthetic library ($BOOKS books) in $BOOKS_DIR" >&2
  (cd "$SERVER_DIR" && cargo run --release -p freelib-import --bin gen-inpx -- \
    --books "$BOOKS" --out "$INPX" --with-files "$BOOKS_DIR" --seed 42) >&2
else
  echo "[real-server] reusing existing synthetic library in $BOOKS_DIR" >&2
fi

export FREELIB_PORT="${FREELIB_REAL_E2E_PORT:-8099}"
export FREELIB_BIND=127.0.0.1
export FREELIB_DATA_DIR="$DATA_DIR"
export FREELIB_CACHE_DIR="$CACHE_DIR"
export FREELIB_BOOKS_DIR="$BOOKS_DIR"
export FREELIB_EXPORT_DIR="$EXPORT_DIR"
export FREELIB_AUTOIMPORT="$INPX"
# No FREELIB_ADMIN_PASSWORD: the server runs in open mode (no login), like the mock backend
# that the rest of the Playwright suite (pnpm test) runs against, so specs need no login step.
export FREELIB_WEB_DIR="$WEB_DIR/dist"
export FREELIB_CALIBRE=none
export RUST_LOG="${RUST_LOG:-info}"

echo "[real-server] starting freelib-server on 127.0.0.1:$FREELIB_PORT (data in $TMP_DIR)" >&2
exec cargo run --release --manifest-path "$SERVER_DIR/Cargo.toml" -p freelib-server
