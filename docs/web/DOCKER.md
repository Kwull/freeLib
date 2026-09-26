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

3. Create the books folder and put your `.inpx` file and archives in it:
   ```bash
   mkdir -p books
   ```
   The container starts as root just long enough to give `data`, `cache` and `export` to the server's user (`PUID`/`PGID`, default 1000:1000), then runs the server as that user. Set `PUID`/`PGID` to your own ids (`id -u`, `id -g`) to keep the files owned by you on the host.

4. Start the service:
   ```bash
   docker compose up -d
   ```

5. Access freeLib at `http://localhost:8080`

## Volumes

The `docker-compose.yml` mounts four volumes:

- **`./books:/books:ro`** (read-only): Library directory where INPX files and archives must be stored.
- **`./data:/data`**: Database and catalog files (`app.db`, `lib_<id>.db`). This is what needs to be backed up.
- **`./cache:/cache`**: Extracted book covers, annotations, and converted books. Safe to delete; will be rebuilt on demand. Bounded by `FREELIB_CACHE_MAX_MB` (default 2 GiB, least recently used files are evicted).
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

Includes Calibre (Debian trixie, Calibre 8.x) for format conversion. Supports:
- EPUB 3 (all devices)
- KEPUB (Kobo devices)
- FB2 (embedded conversion via fb2conv)
- AZW3, MOBI, PDF from FB2 and EPUB books (via Calibre, e.g. for Kindle USB)

For safety Calibre only ever converts EPUB files (freeLib's own FB2 → EPUB output or an original
`.epub`); books in other formats (PDF, DjVu, TXT, DOC, …) are offered as originals.

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

- **Root** (libraries, or the default library directly when there's only one): `http://<host>:8080/opds`
- **Library catalog** (New, Authors, Series, Genres, Search): `http://<host>:8080/opds/<library_id>`
- **Authors list**: `http://<host>:8080/opds/<library_id>/authors`
- **Series list**: `http://<host>:8080/opds/<library_id>/series`

Add these URLs to your e-reader's OPDS client (e.g., Kindle email, Kobo, FBReader). See
[API.md](./API.md#opds-not-under-api) for the full path list.

## Reverse Proxy Setup

Running behind a reverse proxy (Caddy, nginx, Apache)? Ensure the proxy does not buffer
responses to `/api/v1/events` (Server-Sent Events for real-time updates) **and to `/mcp`** (the MCP
endpoint answers with JSON, but switches to an SSE stream when a tool reports progress; treat it
like the events stream: no buffering, long read timeout, `Authorization` header passed through), and set
`FREELIB_TRUST_PROXY=1` so login rate limiting uses the real client address instead of the
proxy's own address. With it, freeLib takes `X-Real-IP` when present, otherwise the **rightmost**
`X-Forwarded-For` entry (the one your proxy appended; anything to its left comes from the client
and can be forged). Only set it when the container is reachable through that one proxy (don't
publish port 8080 to the network), or clients could send the header themselves.

Login attempts are also throttled per user name, so a shared proxy address does not lock
everyone out and rotating addresses does not help an attacker.

Open mode (no `FREELIB_ADMIN_PASSWORD`, no users) only answers requests for `localhost` and IP
addresses, to protect against DNS rebinding. If you reach an open-mode server by name (e.g.
`http://nas.lan:8080`), list the names in `FREELIB_ALLOWED_HOSTS=nas.lan` (comma-separated,
`*.lan` matches sub-domains); requests for other names get `421 Misdirected Request`. Once set, the
list is enforced in login mode too.

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
    proxy_set_header Host $host;
    # the client address as nginx sees it; never pass on client-supplied values
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $remote_addr;
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
| `PUID` / `PGID` | `1000` / `1000` | User and group the server runs as; `/data`, `/cache` and `/export` are given to them on start. Use your host user's ids on a NAS. If the container is started with `--user`, that user is used and no ownership changes are made |
| `FREELIB_PORT` | `8080` | HTTP port (inside container; use port mapping in compose) |
| `FREELIB_BIND` | `0.0.0.0` | Listen address |
| `FREELIB_DATA_DIR` | `/data` | Database directory (`app.db`, `lib_<id>.db`) |
| `FREELIB_BOOKS_DIR` | `/books` | Library root directory |
| `FREELIB_CACHE_DIR` | `/cache` | Cache directory |
| `FREELIB_EXPORT_DIR` | `/export` | Export/send target directory |
| `FREELIB_ADMIN_USER` | `admin` | Admin username created/reset on every start when `FREELIB_ADMIN_PASSWORD` is set |
| `FREELIB_ADMIN_PASSWORD` | unset | Admin password (set this!). Without it and without any users, the server runs in **open mode**: no login, every request acts as admin |
| `FREELIB_AUTOIMPORT` | unset | Comma-separated INPX paths to import on startup |
| `FREELIB_CALIBRE` | `ebook-convert` if on `PATH` | Path to Calibre's `ebook-convert` (full image only); `none` disables Calibre. Calibre older than 6.19 (CVE-2023-46303) is ignored with a warning |
| `FREELIB_CALIBRE_TIMEOUT` | `300` | Seconds before a Calibre conversion is killed |
| `FREELIB_TRUST_PROXY` | unset | `1`: behind exactly one reverse proxy; the client address for login rate limiting is `X-Real-IP`, else the rightmost `X-Forwarded-For` entry (see [Reverse Proxy Setup](#reverse-proxy-setup)) |
| `FREELIB_ALLOWED_HOSTS` | unset | Comma-separated host names accepted besides `localhost` and IP addresses (`*.lan` for sub-domains). Required to reach an **open-mode** server by name (DNS rebinding protection); enforced in every mode once set |
| `FREELIB_PUBLIC_URL` | unset | External base URL, e.g. `https://books.example.org` (no trailing slash). Required for single sign-on; an `https://` URL makes cookies `Secure` |
| `FREELIB_OIDC_ISSUER` | unset | OpenID Connect issuer URL, exactly as in the provider's `/.well-known/openid-configuration` (see [Single sign-on](#single-sign-on-openid-connect)) |
| `FREELIB_OIDC_CLIENT_ID` | unset | Client id registered at the provider (required with the issuer) |
| `FREELIB_OIDC_CLIENT_SECRET` | unset | Client secret; leave unset for a public client (PKCE only) |
| `FREELIB_OIDC_SCOPES` | `openid profile email` | Requested scopes; add `groups` when using `FREELIB_OIDC_ADMIN_GROUP` with Pocket ID or Authentik |
| `FREELIB_OIDC_BUTTON` | `Sign in with SSO` | Text of the sign-in button, e.g. `Sign in with Pocket ID` |
| `FREELIB_OIDC_ADMIN_GROUP` | unset | Group whose members are administrators (everyone else: reader), re-checked at every sign-in |
| `FREELIB_OIDC_AUTO_CREATE` | `true` | Create a reader account at a first sign-in; `false`: only accounts whose owner linked SSO in Settings → Account |
| `FREELIB_OIDC_DISABLE_PASSWORD` | `false` | Password sign-in off in the web app, except for `FREELIB_ADMIN_USER` while `FREELIB_ADMIN_PASSWORD` is set |
| `FREELIB_CACHE_MAX_MB` | `2048` | Size limit of the conversion / cover cache in `/cache`; least recently used files are evicted (`0` = no limit) |
| `FREELIB_WORKERS` | CPU core count | Conversion worker count |
| `FREELIB_WEB_DIR` | unset | Serve the SPA from this folder instead of the embedded copy (development) |
| `FREELIB_CONTACT_EMAIL` | unset | Contact address sent in the User-Agent of Open Library requests (recommended by Open Library) |
| `FREELIB_OPENLIBRARY_URL` | `https://openlibrary.org` | Open Library base URL (tests use a local fake) |
| `FREELIB_MCP_RATE` | `120` | MCP requests per API token and minute |
| `RUST_LOG` | `info` | Log level (debug, info, warn, error) |

## Ratings and Open Library (privacy)

Book lists can show three ratings: your own, the library's (the `LIBRATE`/stars field of the INPX) and
**Open Library**'s (openlibrary.org, average and vote count). For the last one the server looks books up
in the background — about one request a second, books you open, shelve, rate or send first, then the
authors and series you browse, then the rest of the library — and caches the answers in `/data/ratings.db`
(safe to delete; found ratings are refreshed after 90 days).

**What leaves your server:** book titles and author surnames, sent to openlibrary.org with a User-Agent
naming freeLib (and `FREELIB_CONTACT_EMAIL`, if set). Nothing about users. An administrator can switch it
off in **Settings → Server → External ratings**; then nothing is sent (cached ratings are still shown).
Progress is shown there and on the Libraries page.

The "suitable for age" badge and filter (0+, 6+, 12+, 16+, 18+) is a **heuristic** from genres and keywords,
not a verified age rating.

## AI assistants (MCP)

freeLib includes an MCP server at `<your server>/mcp` so Claude and other MCP clients can search your
library, look at your shelves, ratings and history, suggest what to read next (with reasons), rate books,
manage shelves and send books to your devices. An administrator can switch it off in **Settings → Server**.

1. **Settings → Account → API tokens & MCP**: create a token, choose its permissions — `read` (catalog and
   your profile), `write` (shelves and ratings), `send` (Send to Kindle / devices) — and copy it (it is shown
   once). The page shows the MCP URL and ready-made snippets; revoke a token there at any time, and see the
   last 50 tool calls.
2. Connect a client (replace the URL and token):

   **Claude Code**
   ```bash
   claude mcp add --transport http freelib https://books.example.org/mcp --header "Authorization: Bearer fl_…"
   ```

   **Claude Desktop** — custom connectors added in *Settings → Connectors* only support OAuth, so use
   [`mcp-remote`](https://github.com/geelen/mcp-remote) (needs Node.js) in `claude_desktop_config.json`
   (*Settings → Developer → Edit Config*), then restart Claude Desktop:
   ```json
   {
     "mcpServers": {
       "freelib": {
         "command": "npx",
         "args": ["-y", "mcp-remote", "https://books.example.org/mcp", "--header", "Authorization:${FREELIB_AUTH}"],
         "env": { "FREELIB_AUTH": "Bearer fl_…" }
       }
     }
   }
   ```
   (The header value goes through `env` because some clients mangle spaces inside `args`.)

   **Other clients** (Cursor, VS Code, … — Streamable HTTP with headers):
   ```json
   { "mcpServers": { "freelib": { "type": "http", "url": "https://books.example.org/mcp",
     "headers": { "Authorization": "Bearer fl_…" } } } }
   ```
3. Try the prompts `suggest_next_book`, `books_for_kid` (age) or `similar_to` (book id), or just ask
   "what should I read next?".

Use HTTPS for anything beyond your LAN — the token is a password. Tokens only work for `/mcp`, not for the
web app or OPDS. Requests are limited to `FREELIB_MCP_RATE` per token and minute.

## Sending by e-mail

Settings → Mail limits where books can be mailed: **Allowed recipients** (default `*@kindle.com`,
`*@free.kindle.com`; `*` matches any characters, a single `*` allows every address) and
**Mails per user per day** (default 100). The rules apply to every user, administrators included,
so the server's SMTP account cannot be used to mail arbitrary people.

Several books (or a whole series) travel together: up to **25 books and 50 MB per e-mail**
(Amazon's Send to Kindle limits; lower the size if your provider allows less — Gmail: 25 MB).
Temporary SMTP failures are retried automatically (3 times: after 30 s, 2 min, 8 min); each
book's delivery state, with the mail server's reply, is in the Activity panel, and jobs survive
a container restart. **Add the From address to Amazon's Approved Personal Document E-mail List**
(Amazon → Manage Your Content and Devices → Preferences → Personal Document Settings), otherwise
Amazon silently drops the mails. See [DEVICES.md](DEVICES.md) for the default devices, the
metadata and covers in every EPUB, "Send to my phone" and "Open in Books" on iPhone/iPad. Phone
links point at `FREELIB_PUBLIC_URL` when it is set, else at the address the browser used — use
an address the phone can reach (not `localhost`).

## Single sign-on (OpenID Connect)

Users can sign in through an OpenID Connect provider — [Pocket ID](https://pocket-id.org), Authelia,
Authentik, Keycloak, Google, … — in addition to local accounts, which stay as the fallback. The flow is
the authorization code flow with PKCE; the server talks to the provider directly (it needs outbound
HTTPS to it).

### Pocket ID, step by step

1. In Pocket ID, open **Administration → OIDC Clients → Add OIDC Client**.
2. **Name**: `freeLib` (optionally a logo).
3. **Callback URLs**: `https://books.example.org/api/v1/auth/oidc/callback` — your
   `FREELIB_PUBLIC_URL` followed by `/api/v1/auth/oidc/callback`, character for character.
4. **Public client**: either
   * off (confidential client, recommended): keep the **client secret** Pocket ID shows after saving, or
   * on: no secret; freeLib always uses PKCE (leave the PKCE switch on).
5. Optionally restrict who may sign in with **Allowed user groups**, and create a group such as
   `freelib-admins` for administrators (Administration → User Groups).
6. Save; copy the **Client ID** (and secret).
7. Configure freeLib:

```yaml
    environment:
      - FREELIB_ADMIN_PASSWORD=change-me            # keeps a local way in
      - FREELIB_PUBLIC_URL=https://books.example.org
      - FREELIB_OIDC_ISSUER=https://id.example.org  # the Pocket ID URL, no trailing slash
      - FREELIB_OIDC_CLIENT_ID=<client id>
      - FREELIB_OIDC_CLIENT_SECRET=<client secret>  # omit for a public client
      - FREELIB_OIDC_SCOPES=openid profile email groups
      - FREELIB_OIDC_BUTTON=Sign in with Pocket ID
      - FREELIB_OIDC_ADMIN_GROUP=freelib-admins
```

8. Restart. The log says `single sign-on ready` with the redirect URI; the login page shows the button.
   The first sign-in creates an account named after the Pocket ID user name (a reader, or an administrator
   for members of `freelib-admins`).

Existing local users link their provider account in **Settings → Account → Link single sign-on** (and can
unlink it there once they have a password). Administrators see which users are linked in Settings → Users.

### Other providers

* **Authelia**: register a client with `redirect_uris: [https://books.example.org/api/v1/auth/oidc/callback]`,
  `scopes: [openid, profile, email, groups]`, `require_pkce: true`, `pkce_challenge_method: S256`;
  issuer = your Authelia URL. Groups are read from userinfo when the ID token has none.
* **Authentik**: an OAuth2/OpenID provider + application; redirect URI as above; issuer is
  `https://auth.example.org/application/o/<slug>/` (with the trailing slash).
* **Keycloak**: a client with *Standard flow*, valid redirect URI as above; issuer
  `https://kc.example.org/realms/<realm>`. For groups add a *Group Membership* mapper (claim `groups`,
  full group path off).
* **Google**: issuer `https://accounts.google.com`, an OAuth client of type *Web application*. Anybody
  with a Google account can sign in, so set `FREELIB_OIDC_AUTO_CREATE=false` and let users link their
  accounts from Settings → Account.

### Notes

* Accounts are matched by the provider's subject id, never by user name or e-mail: a provider user called
  `admin` becomes `admin (2)`, not your local administrator.
* With single sign-on configured the server never runs in open mode. If there is no administrator,
  set `FREELIB_ADMIN_PASSWORD` or `FREELIB_OIDC_ADMIN_GROUP`.
* `FREELIB_OIDC_DISABLE_PASSWORD=true` hides the password form; the `FREELIB_ADMIN_USER` account can still
  sign in with a password ("Sign in with a local administrator account") while `FREELIB_ADMIN_PASSWORD` is set.
* **OPDS** apps cannot do single sign-on: they keep using HTTP Basic auth with a local password. Users
  whose account was created by single sign-on set one in **Settings → Account** ("Set a password for apps").
* A provider with a private CA: mount its certificate and set `SSL_CERT_FILE` to a bundle containing it.
* Errors are shown on the login page; the server log has the details (`single sign-on failed: …`).

## Troubleshooting

### "Rebuilding the catalog" after an update

When an update changes the catalog format, each library is re-imported from its INPX on start (about a
minute for a Flibusta-size library). The library cannot be browsed until then: the web app shows the
progress and continues by itself when it is done; other, ready libraries can be opened meanwhile.

### Container exits immediately

Check logs:
```bash
docker compose logs freelib
```

Common issues:
- `Permission denied`: Ensure `./data`, `./cache`, and `./export` directories are writable.
- `Port 8080 in use`: Change the port mapping in `docker-compose.yml`.

### Import is slow

The first import of a large INPX takes time (typically seconds to a few minutes; a
Flibusta-size library imports in well under 3 minutes). Monitor progress in the Logs section of
the web UI. A re-import (`{mode: "new"}` or a full re-import) always rebuilds the whole catalog
in the background — there is no incremental/partial mode — but the library keeps serving the old
catalog until the rebuild finishes, so it stays usable throughout. User data (ratings, shelves)
is keyed by a stable book key and survives re-imports.

### Calibre conversion fails

Calibre (full image) requires display server. The container sets `QT_QPA_PLATFORM=offscreen`.
If conversions fail, check server logs and ensure adequate disk space in `/cache`. A startup
warning "older than 6.19" means the installed Calibre is vulnerable and was disabled: use the
current image or install a newer Calibre.

### SSE events not reaching browser (stuck updates)

Your reverse proxy is likely buffering responses. Ensure:
- Proxy buffering is disabled for `/api/v1/events`
- Connection upgrade headers are passed through
- Timeouts are high (SSE connections are long-lived)

See [Reverse Proxy Setup](#reverse-proxy-setup) above.
