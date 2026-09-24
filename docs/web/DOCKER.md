# Docker Deployment Guide

This guide covers running freeLib Web Edition with Docker.

## Quick Start

1. Clone the repository:
   ```bash
   git clone https://github.com/kwull/freeLib.git
   cd freeLib/docker
   ```

2. Copy and customize the environment file:
   ```bash
   cp .env.example .env
   # Edit .env and set FREELIB_ADMIN_PASSWORD
   ```

3. Start the service:
   ```bash
   docker compose up -d
   ```

4. Access freeLib at `http://localhost:8080`

## Volumes

The `docker-compose.yml` mounts four volumes:

- **`./books:/books:ro`** (read-only): Library directory where INPX files and archives must be stored.
- **`./data:/data`**: Database and catalog files (`app.db`, `lib_<id>.db`). This is what needs to be backed up.
- **`./cache:/cache`**: Extracted book covers, annotations, and converted books. Safe to delete; will be rebuilt on demand.
- **`./export:/export`**: Target folder for the "Server folder" device (export/send operations).

## Adding Libraries

### Method 1: Web UI

1. Copy your INPX file and archives to the `./books` directory on the host.
2. Open freeLib in your browser: `http://localhost:8080`
3. Log in with the admin password you set in `.env`.
4. Go to **Libraries** → **Add** and select the INPX file.

### Method 2: Automatic Import

Set `FREELIB_AUTOIMPORT` in `.env` to auto-import on startup:

```env
FREELIB_AUTOIMPORT=/books/library.inpx
```

Multiple libraries can be comma-separated:

```env
FREELIB_AUTOIMPORT=/books/lib1.inpx,/books/lib2.inpx
```

## Image Variants

### `full` (default)

Includes Calibre for format conversion. Supports:
- EPUB 3 (all devices)
- KEPUB (Kobo devices)
- FB2 (embedded conversion via fb2conv)
- AZW3, MOBI, PDF (via Calibre for Kindle USB)
- Other formats (via Calibre)

Use this for full device support. Larger image (~1.5 GB).

### `slim`

Minimal image with only freeLib and essential tools. Supports:
- EPUB 3 (all devices)
- KEPUB (Kobo devices)
- FB2 (embedded conversion via fb2conv)

Suitable for Send to Kindle (email) and Apple Books. Smaller image (~300 MB).

To use slim:

```yaml
# docker-compose.yml
services:
  freelib:
    image: ghcr.io/kwull/freelib:slim
    # ... rest of config
```

Or build locally:

```bash
docker compose -f - <<'EOF'
services:
  freelib:
    build:
      context: ..
      dockerfile: docker/Dockerfile
      target: slim
    # ... rest of config
EOF
```

## OPDS Feed

The server exposes OPDS 1.2 feeds for e-readers:

- **OPDS catalog**: `http://<host>:8080/opds/`
- **Library catalog**: `http://<host>:8080/opds/catalog/<library_id>`
- **Authors list**: `http://<host>:8080/opds/authors/<library_id>`
- **Series list**: `http://<host>:8080/opds/series/<library_id>`

Add these URLs to your e-reader's OPDS client (e.g., Kindle email, Kobo, FBReader).

## Reverse Proxy Setup

Running behind a reverse proxy (Caddy, nginx, Apache)? Ensure the proxy does not buffer
responses to `/api/v1/events` (Server-Sent Events for real-time updates).

### Caddy

```caddy
reverse_proxy localhost:8080 {
    # SSE-friendly: no buffering
    header_up Connection "upgrade"
}
```

### Nginx

```nginx
location / {
    proxy_pass http://localhost:8080;
    proxy_buffering off;
    proxy_http_version 1.1;
    proxy_set_header Connection "";
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

### Apache

```apache
ProxyPass / http://localhost:8080/
ProxyPassReverse / http://localhost:8080/
SetEnv proxy-sendcl 1
# For SSE, add:
# Header edit Transfer-Encoding "chunked" ""
```

## Backups

freeLib maintains two databases:

- **`/data/app.db`**: User accounts, sessions, shelves, ratings, settings. Contains all user state.
- **`/data/lib_<id>.db`**: Read-only catalog for each library. Rebuilt on re-import.

To back up:

```bash
# Full backup (app state + current catalogs)
tar czf freeLib-backup-$(date +%Y%m%d).tar.gz ./data

# App state only (catalogs can be re-imported)
tar czf freeLib-app-backup-$(date +%Y%m%d).tar.gz ./data/app.db
```

To restore:

```bash
# Stop the service
docker compose down

# Restore backup
tar xzf freeLib-backup-YYYYMMDD.tar.gz

# Restart
docker compose up -d
```

## Environment Variables

All environment variables from [ARCHITECTURE.md](./ARCHITECTURE.md#runtime-configuration-environment) are available:

| Variable | Default | Meaning |
|----------|---------|---------|
| `FREELIB_PORT` | `8080` | HTTP port (inside container; use port mapping in compose) |
| `FREELIB_DATA_DIR` | `/data` | Database directory |
| `FREELIB_BOOKS_DIR` | `/books` | Library root directory |
| `FREELIB_CACHE_DIR` | `/cache` | Cache directory |
| `FREELIB_EXPORT_DIR` | `/export` | Export/send target directory |
| `FREELIB_ADMIN_PASSWORD` | unset | Admin password (set this!) |
| `FREELIB_AUTOIMPORT` | unset | Comma-separated INPX paths to import on startup |
| `FREELIB_CALIBRE` | `ebook-convert` | Path to Calibre's ebook-convert (full image only) |
| `RUST_LOG` | `info` | Log level (debug, info, warn, error) |

## Troubleshooting

### Container exits immediately

Check logs:
```bash
docker compose logs freelib
```

Common issues:
- `Permission denied`: Ensure `./data`, `./cache`, and `./export` directories are writable.
- `Port 8080 in use`: Change the port mapping in `docker-compose.yml`.

### Import is slow

The first import of a large INPX takes time. Monitor progress in the Logs section of the web UI.
Subsequent imports are faster (incremental updates).

### Calibre conversion fails

Calibre (full image) requires display server. The container sets `QT_QPA_PLATFORM=offscreen`.
If conversions fail, check server logs and ensure adequate disk space in `/cache`.

### SSE events not reaching browser (stuck updates)

Your reverse proxy is likely buffering responses. Ensure:
- Proxy buffering is disabled for `/api/v1/events`
- Connection upgrade headers are passed through
- Timeouts are high (SSE connections are long-lived)

See [Reverse Proxy Setup](#reverse-proxy-setup) above.
