//! Startup (directories, app.db, admin bootstrap, libraries, auto-import, background tasks)
//! and the HTTP router.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Uri};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
use tower_http::trace::TraceLayer;

use crate::calibre::Calibre;
use crate::config::Config;
use crate::db::{self, AppDb, LibraryRow, User};
use crate::state::AppState;
use crate::{api, auth, importer, opds, security, spa};

/// Opens everything and starts background tasks. Must run inside a Tokio runtime.
pub async fn init(cfg: Config) -> anyhow::Result<AppState> {
    for d in [&cfg.data_dir, &cfg.cache_dir] {
        ensure_writable_dir(d)?;
    }
    crate::cache::clean_on_start(&cfg.cache_dir);
    let db = AppDb::open(&cfg.data_dir.join("app.db"))?;
    let secrets = std::sync::Arc::new(crate::secrets::Secrets::load(&cfg)?);
    {
        let rep = crate::secrets::migrate(&db.lock(), &secrets)?;
        tracing::info!(
            "stored secrets are encrypted with key {} from {}",
            secrets.key_id(),
            secrets.source
        );
        if rep.encrypted > 0 {
            tracing::info!(
                "encrypted {} stored secret(s) that were plain text",
                rep.encrypted
            );
        }
        if rep.rotated > 0 {
            tracing::info!(
                "re-encrypted {} stored secret(s) with the new key; FREELIB_SECRET_KEY_OLD can be removed",
                rep.rotated
            );
        }
    }
    let calibre = match &cfg.calibre {
        Some(p) => Calibre::detect(p).await,
        None => None,
    };
    match &calibre {
        Some(c) => tracing::info!(
            "Calibre {} at {}",
            c.version.as_deref().unwrap_or("?"),
            c.path.display()
        ),
        None => tracing::info!("Calibre not found: AZW3/MOBI/PDF disabled"),
    }
    let oidc = match &cfg.oidc {
        Some(o) => Some(
            crate::oidc::Provider::new(o.clone(), cfg.public_url.as_deref())
                .map_err(|e| anyhow::anyhow!("single sign-on: {e}"))?,
        ),
        None => None,
    };
    let http: std::sync::Arc<dyn crate::extrating::openlibrary::HttpGet> = std::sync::Arc::new(
        crate::extrating::openlibrary::ReqwestGet::new(&crate::extrating::openlibrary::user_agent(
            cfg.contact_email.as_deref(),
        ))
        .map_err(|e| anyhow::anyhow!("HTTP client: {e}"))?,
    );
    let ext = std::sync::Arc::new(crate::extrating::ExtRatings::open(
        &cfg.data_dir.join("ratings.db"),
        &cfg.openlibrary_url,
        http,
        cfg.ext_interval,
    )?);
    {
        let c = db.lock();
        let on = db::get_setting::<db::ExtRatingsConfig>(&c, "externalRatings")?.enabled;
        ext.set_enabled(on);
    }
    let st = AppState::new(cfg, db, calibre, oidc, ext, secrets);
    match crate::oauth::issuer(&st) {
        Some(iss) => {
            tracing::info!("OAuth for MCP clients: add {iss}/mcp as a custom connector in Claude")
        }
        None => tracing::info!(
            "OAuth for MCP clients is off (set FREELIB_PUBLIC_URL to this server's https:// address)"
        ),
    }
    bootstrap_users(&st)?;
    if let Some(p) = st.oidc.clone() {
        tokio::spawn(async move { p.probe().await });
    }
    {
        let c = st.db.lock();
        db::seed_devices(&c)?;
    }
    let rows = {
        let c = st.db.lock();
        db::list_libraries(&c)?
    };
    for r in &rows {
        let rt = st.add_lib(r.id);
        if let Some(cat) = rt.handle.get() {
            importer::warm(&st, &rt, cat);
        }
    }
    autoimport(&st).await;
    reimport_outdated(&st, &rows).await;
    spawn_cleanup(&st);
    if st.cfg.ext_worker {
        tokio::spawn(st.ext.clone().run(st.clone()));
    }
    Ok(st)
}

fn bootstrap_users(st: &AppState) -> anyhow::Result<()> {
    let c = st.db.lock();
    if let Some(pw) = &st.cfg.admin_password {
        let name = st.cfg.admin_user.clone();
        let hash = auth::hash_password(pw, st.cfg.fast_password_hash)?;
        match db::user_with_hash(&c, &name)? {
            Some((u, old)) => {
                if !auth::verify_password(pw, &old) {
                    db::update_user(&c, u.id, Some(&hash), Some("admin"))?;
                    tracing::info!(
                        "admin password of '{name}' updated from FREELIB_ADMIN_PASSWORD"
                    );
                } else if !u.is_admin() {
                    db::update_user(&c, u.id, None, Some("admin"))?;
                }
            }
            None => {
                db::insert_user(&c, &name, &hash, "admin")?;
                tracing::info!("created administrator '{name}'");
            }
        }
    }
    let no_admin = db::count_admins(&c)? == 0;
    // with single sign-on configured the server always requires a login
    let open = no_admin && st.oidc.is_none();
    st.open_mode.store(open, Ordering::Relaxed);
    if open {
        tracing::warn!(
            "no users and no FREELIB_ADMIN_PASSWORD: running in OPEN MODE (no login, everyone is admin)"
        );
    } else if no_admin {
        tracing::warn!(
            "no administrator yet: set FREELIB_ADMIN_PASSWORD, or FREELIB_OIDC_ADMIN_GROUP to make \
             members of that group administrators"
        );
    }
    Ok(())
}

/// The user that owns jobs started by the server itself.
fn system_user(st: &AppState) -> User {
    let c = st.db.lock();
    db::list_users(&c)
        .ok()
        .and_then(|u| u.into_iter().find(|u| u.is_admin()))
        .unwrap_or_else(User::open_mode_admin)
}

/// Libraries whose catalog file exists but cannot be opened (written by another catalog schema
/// version, e.g. before an upgrade) are re-imported from their INPX in the background.
async fn reimport_outdated(st: &AppState, rows: &[LibraryRow]) {
    for r in rows {
        let Ok(rt) = st.lib(r.id) else { continue };
        if rt.handle.get().is_some() || !rt.handle.path().is_file() || r.inpx.is_none() {
            continue;
        }
        if rt
            .import
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            continue; // FREELIB_AUTOIMPORT already started it
        }
        tracing::info!(
            "library {} has an outdated or unreadable catalog: re-importing",
            r.id
        );
        let owner = system_user(st);
        if let Err(e) = importer::start_with_reason(st, r.id, &owner, Some("upgrade")).await {
            tracing::warn!("re-import of library {} failed to start: {e}", r.id);
        }
    }
}

async fn autoimport(st: &AppState) {
    if st.cfg.autoimport.is_empty() {
        return;
    }
    let owner = system_user(st);
    for inpx in st.cfg.autoimport.clone() {
        let Ok(inpx) = inpx.canonicalize() else {
            tracing::warn!("FREELIB_AUTOIMPORT: {} not found", inpx.display());
            continue;
        };
        let inpx_s = inpx.to_string_lossy().into_owned();
        let existing = {
            let c = st.db.lock();
            db::list_libraries(&c)
                .unwrap_or_default()
                .into_iter()
                .find(|l| l.inpx.as_deref() == Some(inpx_s.as_str()))
        };
        let id = match existing {
            Some(l) => {
                if st.lib(l.id).ok().and_then(|rt| rt.handle.get()).is_some() {
                    continue; // already imported
                }
                l.id
            }
            None => {
                let name = inpx_name(&inpx);
                let row = LibraryRow {
                    id: 0,
                    name,
                    path: library_dir_for(&inpx).to_string_lossy().into_owned(),
                    inpx: Some(inpx_s.clone()),
                    first_author_only: false,
                    skip_deleted: false,
                    is_default: false,
                };
                let id = {
                    let c = st.db.lock();
                    match db::insert_library(&c, &row) {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::warn!("FREELIB_AUTOIMPORT: {e}");
                            continue;
                        }
                    }
                };
                st.add_lib(id);
                id
            }
        };
        tracing::info!("auto-importing {inpx_s} as library {id}");
        if let Err(e) = importer::start_with_reason(st, id, &owner, Some("autoimport")).await {
            tracing::warn!("auto-import of {inpx_s} failed to start: {e}");
        }
    }
}

/// Folder with the archives of an INPX: its own folder, or a sibling folder named like the INPX
/// (`books/flibusta.inpx` + `books/flibusta/`) when that exists.
fn library_dir_for(inpx: &Path) -> PathBuf {
    let parent = inpx.parent().unwrap_or(Path::new("/"));
    if let Some(stem) = inpx.file_stem() {
        let sib = parent.join(stem);
        if sib.is_dir() {
            return sib;
        }
    }
    parent.to_path_buf()
}

/// Library name: the collection name from the INPX, else the file name.
fn inpx_name(inpx: &Path) -> String {
    freelib_import::inpx::read_info(inpx)
        .ok()
        .and_then(|i| i.collection_name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            inpx.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Library".into())
        })
}

/// Every 10 minutes: expire jobs and their files, remove stale temp files, bound the cache.
fn spawn_cleanup(st: &AppState) {
    let st = st.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(600));
        loop {
            tick.tick().await;
            if st.shutdown.load(Ordering::Relaxed) {
                break;
            }
            let ttl = st.cfg.job_file_ttl;
            for d in st.jobs.expire(ttl.as_secs() as i64) {
                let _ = tokio::fs::remove_dir_all(d).await;
            }
            let cache = st.cfg.cache_dir.clone();
            let max = st.cfg.cache_max_bytes;
            let _ = tokio::task::spawn_blocking(move || {
                remove_older(&cache.join("tmp"), Duration::from_secs(3600));
                remove_older(&cache.join("jobs"), ttl + Duration::from_secs(3600));
                crate::cache::evict(&cache, max);
            })
            .await;
            crate::oauth::cleanup(&st).await;
        }
    });
}

fn remove_older(dir: &Path, age: Duration) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|el| el > age);
        if old {
            let p: PathBuf = e.path();
            let _ = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
        }
    }
}

async fn fallback(
    State(st): State<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if uri.path().starts_with("/opds_")
        && let Some(r) = opds::legacy_redirect(uri.path(), uri.query())
    {
        return r;
    }
    if uri.path().starts_with("/opds/") {
        return (axum::http::StatusCode::NOT_FOUND, "not found").into_response();
    }
    spa::serve(State(st), method, uri, headers).await
}

pub fn router(st: AppState) -> Router {
    let compress = DefaultPredicate::new()
        .and(NotForContentType::const_new("application/epub+zip"))
        .and(NotForContentType::const_new("application/zip"))
        .and(NotForContentType::const_new(
            "application/x-mobipocket-ebook",
        ))
        .and(NotForContentType::const_new("application/vnd.amazon.ebook"))
        .and(NotForContentType::const_new("application/pdf"))
        .and(NotForContentType::const_new("image/vnd.djvu"))
        .and(NotForContentType::const_new("font/"));
    Router::new()
        .nest(
            "/api/v1",
            api::router().layer(middleware::from_fn(security::csrf_layer)),
        )
        .merge(opds::router().layer(middleware::from_fn_with_state(st.clone(), opds::gate)))
        .merge(crate::mcp::router(st.clone()))
        .merge(crate::oauth::router())
        .fallback(fallback)
        .layer(
            CompressionLayer::new()
                .br(true)
                .gzip(true)
                .quality(tower_http::CompressionLevel::Precise(4))
                .compress_when(compress),
        )
        .layer(middleware::from_fn_with_state(
            st.clone(),
            security::host_guard,
        ))
        .layer(middleware::from_fn(security::security_headers))
        // spans carry the path only: query strings may hold sign-in codes
        .layer(TraceLayer::new_for_http().make_span_with(
            |req: &axum::http::Request<axum::body::Body>| {
                tracing::debug_span!("request", method = %req.method(), path = %req.uri().path())
            },
        ))
        .with_state(st)
}

/// Creates `dir` if needed and checks that this process can write to it, so a
/// wrongly owned volume fails with an actionable message instead of SQLite's
/// "unable to open database file".
fn ensure_writable_dir(dir: &Path) -> anyhow::Result<()> {
    let probe = dir.join(".freelib-write-test");
    let res = std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::write(&probe, b""))
        .and_then(|_| std::fs::remove_file(&probe));
    res.map_err(|e| {
        // SAFETY: getuid/getgid have no preconditions and cannot fail.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        anyhow::anyhow!(
            "{} is not writable by uid {uid}, gid {gid}: {e}. In Docker, set PUID/PGID to the \
             owner of the host folder, or give that folder to uid {uid}: \
             `sudo chown -R {uid}:{gid} <host folder>`",
            dir.display()
        )
    })
}
