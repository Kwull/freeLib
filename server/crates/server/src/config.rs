//! Runtime configuration from the environment (see `docs/web/ARCHITECTURE.md`).

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::secrets::Redacted;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: IpAddr,
    pub port: u16,
    pub data_dir: PathBuf,
    pub books_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub export_dir: PathBuf,
    pub admin_user: String,
    pub admin_password: Option<Redacted>,
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
    /// External base URL (`FREELIB_PUBLIC_URL`, no trailing slash), e.g.
    /// `https://books.example.org`. Needed for single sign-on (the redirect URI).
    pub public_url: Option<String>,
    /// OpenID Connect sign-in (`FREELIB_OIDC_*`), when configured.
    pub oidc: Option<OidcConfig>,
    /// Open Library base URL (`FREELIB_OPENLIBRARY_URL`, default `https://openlibrary.org`).
    pub openlibrary_url: String,
    /// Contact e-mail sent in the User-Agent of Open Library requests (`FREELIB_CONTACT_EMAIL`).
    pub contact_email: Option<String>,
    /// Minimum time between two Open Library requests (1 s).
    pub ext_interval: Duration,
    /// Run the background rating enrichment worker (off in [`Config::for_dir`]).
    pub ext_worker: bool,
    /// MCP requests per token and minute (`FREELIB_MCP_RATE`, default 120).
    pub mcp_rate_per_min: u32,
    /// Key for secrets stored in app.db (`FREELIB_SECRET_KEY`: 64 hex digits or base64 of 32
    /// bytes). See [`crate::secrets`].
    pub secret_key: Option<Redacted>,
    /// File holding that key (`FREELIB_SECRET_KEY_FILE`, e.g. a Docker secret).
    pub secret_key_file: Option<PathBuf>,
    /// The previous key during a rotation (`FREELIB_SECRET_KEY_OLD`).
    pub secret_key_old: Option<Redacted>,
    /// Hosts trusted for OAuth clients of the MCP endpoint (`FREELIB_OAUTH_CLIENT_HOSTS`,
    /// default `claude.ai, claude.com`): their `https://` redirect URIs and Client ID Metadata
    /// Documents are accepted. `*` trusts every public host. Loopback redirect URIs of native
    /// apps are always accepted.
    pub oauth_client_hosts: Vec<String>,
    /// Client ID Metadata Documents known without fetching them (URL → JSON); tests only.
    pub oauth_client_docs: Vec<(String, String)>,
}

/// `FREELIB_OIDC_*` (see docs/web/DOCKER.md "Single sign-on").
#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    /// Empty for public clients (PKCE only).
    pub client_secret: Option<Redacted>,
    pub scopes: Vec<String>,
    /// Text of the sign-in button.
    pub button: String,
    /// Members of this group (`groups` claim) are administrators, everybody else a reader.
    pub admin_group: Option<String>,
    /// Create a reader account on the first sign-in of an unknown identity.
    pub auto_create: bool,
    /// Refuse password sign-in in the web app, except for the `FREELIB_ADMIN_USER` account
    /// while `FREELIB_ADMIN_PASSWORD` is set.
    pub disable_password: bool,
}

impl OidcConfig {
    pub fn new(issuer: &str, client_id: &str) -> OidcConfig {
        OidcConfig {
            issuer: issuer.to_string(),
            client_id: client_id.to_string(),
            client_secret: None,
            scopes: default_scopes(),
            button: "Sign in with SSO".into(),
            admin_group: None,
            auto_create: true,
            disable_password: false,
        }
    }
}

fn default_scopes() -> Vec<String> {
    ["openid", "profile", "email"].map(String::from).to_vec()
}

fn flag(name: &str, default: bool) -> bool {
    match env(name) {
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        None => default,
    }
}

/// Reads `FREELIB_OIDC_*`. `Err` for a half-done configuration (the server refuses to start
/// rather than silently running without the sign-in the operator asked for).
fn oidc_from_env() -> Result<Option<OidcConfig>, String> {
    let issuer = env("FREELIB_OIDC_ISSUER");
    let client = env("FREELIB_OIDC_CLIENT_ID");
    let (issuer, client) = match (issuer, client) {
        (None, None) => return Ok(None),
        (Some(i), Some(c)) => (i, c),
        _ => {
            return Err(
                "FREELIB_OIDC_ISSUER and FREELIB_OIDC_CLIENT_ID must be set together".into(),
            );
        }
    };
    let mut o = OidcConfig::new(&issuer, &client);
    o.client_secret = env("FREELIB_OIDC_CLIENT_SECRET").map(Redacted);
    if let Some(s) = env("FREELIB_OIDC_SCOPES") {
        o.scopes = s
            .split([' ', ','])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    if !o.scopes.iter().any(|s| s == "openid") {
        o.scopes.insert(0, "openid".into());
    }
    if let Some(b) = env("FREELIB_OIDC_BUTTON") {
        o.button = b;
    }
    o.admin_group = env("FREELIB_OIDC_ADMIN_GROUP");
    o.auto_create = flag("FREELIB_OIDC_AUTO_CREATE", true);
    o.disable_password = flag("FREELIB_OIDC_DISABLE_PASSWORD", false);
    Ok(Some(o))
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
    /// Whether cookies get the `Secure` flag regardless of the request (`FREELIB_PUBLIC_URL`
    /// is `https://…`).
    pub fn public_https(&self) -> bool {
        self.public_url
            .as_deref()
            .is_some_and(|u| u.to_ascii_lowercase().starts_with("https://"))
    }

    /// Reads the configuration; exits the process on a broken single sign-on setup.
    pub fn from_env() -> Config {
        match Self::try_from_env() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("configuration error: {e}");
                std::process::exit(2);
            }
        }
    }

    pub fn try_from_env() -> Result<Config, String> {
        let calibre = match env("FREELIB_CALIBRE") {
            // explicit "none"/"off" disables Calibre
            Some(v) if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("off") => None,
            Some(v) => which(&v).or(Some(PathBuf::from(v))),
            None => which("ebook-convert"),
        };
        Ok(Config {
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
                .filter(|s| !s.is_empty())
                .map(Redacted),
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
            public_url: env("FREELIB_PUBLIC_URL").map(|u| u.trim_end_matches('/').to_string()),
            oidc: oidc_from_env()?,
            openlibrary_url: env("FREELIB_OPENLIBRARY_URL")
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| "https://openlibrary.org".into()),
            contact_email: env("FREELIB_CONTACT_EMAIL")
                .filter(|e| e.contains('@') && e.len() < 200),
            ext_interval: Duration::from_secs(1),
            ext_worker: true,
            mcp_rate_per_min: env("FREELIB_MCP_RATE")
                .and_then(|s| s.parse().ok())
                .filter(|&n: &u32| n > 0)
                .unwrap_or(120),
            secret_key: env("FREELIB_SECRET_KEY").map(Redacted),
            secret_key_file: env("FREELIB_SECRET_KEY_FILE").map(PathBuf::from),
            secret_key_old: env("FREELIB_SECRET_KEY_OLD").map(Redacted),
            oauth_client_hosts: env("FREELIB_OAUTH_CLIENT_HOSTS")
                .map(|s| parse_hosts(&s))
                .unwrap_or_else(default_client_hosts),
            oauth_client_docs: Vec::new(),
        })
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
            public_url: None,
            oidc: None,
            // never the real service in tests
            openlibrary_url: "http://127.0.0.1:9".into(),
            contact_email: None,
            ext_interval: Duration::from_millis(1),
            ext_worker: false,
            mcp_rate_per_min: 120,
            secret_key: None,
            secret_key_file: None,
            secret_key_old: None,
            oauth_client_hosts: default_client_hosts(),
            oauth_client_docs: Vec::new(),
        }
    }
}

/// Claude (claude.ai on the web, Claude Desktop and mobile, whose OAuth callback is
/// `https://claude.ai/api/mcp/auth_callback`).
pub fn default_client_hosts() -> Vec<String> {
    vec!["claude.ai".into(), "claude.com".into()]
}
