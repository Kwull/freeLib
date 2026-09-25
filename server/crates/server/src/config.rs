//! Runtime configuration from the environment (see `docs/web/ARCHITECTURE.md`).

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: IpAddr,
    pub port: u16,
    pub data_dir: PathBuf,
    pub books_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub export_dir: PathBuf,
    pub admin_user: String,
    pub admin_password: Option<String>,
    pub autoimport: Vec<PathBuf>,
    /// Calibre `ebook-convert`, when available.
    pub calibre: Option<PathBuf>,
    pub web_dir: Option<PathBuf>,
    pub workers: usize,
    /// Use `X-Forwarded-For` / `X-Real-IP` for the client address (login rate limiting).
    pub trust_proxy: bool,
    pub calibre_timeout: Duration,
    pub sse_heartbeat: Duration,
    /// Cheap password hashing parameters (tests only).
    pub fast_password_hash: bool,
    /// Keep files produced by jobs this long.
    pub job_file_ttl: Duration,
    /// Size bound of the conversion/cover/annotation cache (`FREELIB_CACHE_MAX_MB`, 0 = none).
    pub cache_max_bytes: u64,
    /// Host names accepted in the `Host` header besides `localhost` and IP literals
    /// (`FREELIB_ALLOWED_HOSTS`, lower-case; `*.example.org` matches sub-domains). Enforced in
    /// open mode always (DNS rebinding) and in every mode when the list is not empty.
    pub allowed_hosts: Vec<String>,
    /// Queued + running send/export/download jobs per user.
    pub max_jobs_per_user: usize,
}

/// Parses `FREELIB_ALLOWED_HOSTS` (`a.example.org, *.example.net:8080`): lower-case host names,
/// ports dropped.
pub fn parse_hosts(s: &str) -> Vec<String> {
    s.split(',')
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .map(|h| crate::security::strip_port(&h).to_string())
        .collect()
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `/data` in the container, `./data` during development (when `/data` does not exist).
fn dir_default(var: &str, container: &str, dev: &str) -> PathBuf {
    if let Some(v) = env(var) {
        return PathBuf::from(v);
    }
    if Path::new(container).is_dir() {
        PathBuf::from(container)
    } else {
        PathBuf::from(dev)
    }
}

/// Finds an executable on `PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|c| c.is_file())
}

impl Config {
    pub fn from_env() -> Config {
        let calibre = match env("FREELIB_CALIBRE") {
            // explicit "none"/"off" disables Calibre
            Some(v) if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("off") => None,
            Some(v) => which(&v).or(Some(PathBuf::from(v))),
            None => which("ebook-convert"),
        };
        Config {
            bind: env("FREELIB_BIND")
                .and_then(|s| s.parse().ok())
                .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            port: env("FREELIB_PORT")
                .and_then(|s| s.parse().ok())
                .unwrap_or(8080),
            data_dir: dir_default("FREELIB_DATA_DIR", "/data", "./data"),
            books_dir: dir_default("FREELIB_BOOKS_DIR", "/books", "./books"),
            cache_dir: dir_default("FREELIB_CACHE_DIR", "/cache", "./cache"),
            export_dir: dir_default("FREELIB_EXPORT_DIR", "/export", "./export"),
            admin_user: env("FREELIB_ADMIN_USER").unwrap_or_else(|| "admin".into()),
            admin_password: std::env::var("FREELIB_ADMIN_PASSWORD")
                .ok()
                .filter(|s| !s.is_empty()),
            autoimport: env("FREELIB_AUTOIMPORT")
                .map(|s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default(),
            calibre,
            web_dir: env("FREELIB_WEB_DIR").map(PathBuf::from),
            workers: env("FREELIB_WORKERS")
                .and_then(|s| s.parse().ok())
                .filter(|&n: &usize| n > 0)
                .unwrap_or_else(|| {
                    std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(2)
                }),
            trust_proxy: env("FREELIB_TRUST_PROXY")
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
            calibre_timeout: Duration::from_secs(
                env("FREELIB_CALIBRE_TIMEOUT")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(300),
            ),
            sse_heartbeat: Duration::from_secs(25),
            fast_password_hash: false,
            job_file_ttl: Duration::from_secs(24 * 3600),
            cache_max_bytes: env("FREELIB_CACHE_MAX_MB")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(2048)
                .saturating_mul(1024 * 1024),
            allowed_hosts: env("FREELIB_ALLOWED_HOSTS")
                .map(|s| parse_hosts(&s))
                .unwrap_or_default(),
            max_jobs_per_user: 5,
        }
    }

    /// A configuration rooted in one directory (tests, tools).
    pub fn for_dir(root: &Path) -> Config {
        Config {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            data_dir: root.join("data"),
            books_dir: root.join("books"),
            cache_dir: root.join("cache"),
            export_dir: root.join("export"),
            admin_user: "admin".into(),
            admin_password: None,
            autoimport: Vec::new(),
            calibre: None,
            web_dir: None,
            workers: 2,
            trust_proxy: false,
            calibre_timeout: Duration::from_secs(60),
            sse_heartbeat: Duration::from_secs(25),
            fast_password_hash: true,
            job_file_ttl: Duration::from_secs(24 * 3600),
            cache_max_bytes: 2048 * 1024 * 1024,
            allowed_hosts: Vec::new(),
            max_jobs_per_user: 5,
        }
    }
}
