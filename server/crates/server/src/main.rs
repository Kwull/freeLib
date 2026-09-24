//! `freelib-server [serve]`, `freelib-server healthcheck`, `freelib-server migrate-qt <freeLib.sqlite>`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use freelib_server::{Config, db};
use tracing_subscriber::EnvFilter;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("serve") => serve(),
        Some("healthcheck") => std::process::exit(healthcheck()),
        Some("migrate-qt") => match args.get(1) {
            Some(p) => std::process::exit(migrate_qt(PathBuf::from(p))),
            None => usage(),
        },
        Some("--version") | Some("-V") => println!("freelib-server {}", env!("CARGO_PKG_VERSION")),
        _ => usage(),
    }
}

fn usage() {
    eprintln!(
        "usage: freelib-server [serve | healthcheck | migrate-qt <freeLib.sqlite> | --version]"
    );
    std::process::exit(2);
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn serve() {
    init_tracing();
    let cfg = Config::from_env();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        let addr = SocketAddr::new(cfg.bind, cfg.port);
        tracing::info!(
            "freelib-server {} starting: data {}, books {}, cache {}",
            env!("CARGO_PKG_VERSION"),
            cfg.data_dir.display(),
            cfg.books_dir.display(),
            cfg.cache_dir.display()
        );
        let st = match freelib_server::init(cfg).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("startup failed: {e:#}");
                std::process::exit(1);
            }
        };
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("cannot listen on {addr}: {e}");
                std::process::exit(1);
            }
        };
        tracing::info!("listening on http://{addr}");
        let app = freelib_server::router(st.clone());
        let st2 = st.clone();
        let shutdown = async move {
            shutdown_signal().await;
            tracing::info!("shutting down");
            st2.shutdown.store(true, Ordering::Relaxed);
            // SSE connections never end on their own: give requests 5 s, then exit
            tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                std::process::exit(0);
            });
        };
        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown)
        .await
        {
            tracing::error!("server error: {e}");
        }
    });
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

/// GET /api/v1/session on localhost; exit code 0 when it answers 200.
fn healthcheck() -> i32 {
    let port: u16 = std::env::var("FREELIB_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
        return 1;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let req = format!(
        "GET /api/v1/session HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    if s.write_all(req.as_bytes()).is_err() {
        return 1;
    }
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..n]);
    if head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200") {
        0
    } else {
        1
    }
}

fn migrate_qt(path: PathBuf) -> i32 {
    init_tracing();
    let cfg = Config::from_env();
    if let Err(e) = std::fs::create_dir_all(&cfg.data_dir) {
        eprintln!("cannot create {}: {e}", cfg.data_dir.display());
        return 1;
    }
    let appdb = match db::AppDb::open(&cfg.data_dir.join("app.db")) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot open app.db: {e:#}");
            return 1;
        }
    };
    let m = match freelib_import::migrate::read_qt_database(&path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("cannot read {}: {e}", path.display());
            return 1;
        }
    };
    let mut c = appdb.lock();
    let user_id = db::list_users(&c)
        .ok()
        .and_then(|u| u.into_iter().find(|u| u.is_admin()))
        .map(|u| u.id)
        .unwrap_or(0);
    match m.apply(&mut c, user_id) {
        Ok(s) => {
            println!(
                "migrated {} libraries ({} new), {} shelves with {} books, {} ratings; skipped {} author and {} series tags",
                m.libraries.len(),
                s.libraries_created,
                s.shelves_created,
                s.shelf_books,
                s.ratings,
                m.skipped_author_tags,
                m.skipped_series_tags
            );
            for l in &m.libraries {
                if !PathBuf::from(&l.path).starts_with(&cfg.books_dir) {
                    println!(
                        "note: library '{}' path {} is outside FREELIB_BOOKS_DIR; fix it in the web UI",
                        l.name, l.path
                    );
                }
            }
            println!(
                "start the server and run a full import of each library to attach shelves and ratings"
            );
            0
        }
        Err(e) => {
            eprintln!("migration failed: {e}");
            1
        }
    }
}
