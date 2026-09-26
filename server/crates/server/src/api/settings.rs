use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{self, Admin, Auth};
use crate::db::{self, ExtRatingsConfig, McpConfig, OpdsConfig, SmtpConfig, User};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const PREFS_LIMIT: usize = 64 * 1024;

/// Totals of the Open Library lookups over all libraries (Settings → Server).
fn ext_status(st: &AppState, cfg: &ExtRatingsConfig) -> Value {
    let libs: Vec<i64> = st
        .libs
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .copied()
        .collect();
    let (mut looked, mut found, mut rated, mut total) = (0u64, 0u64, 0u64, 0i64);
    for id in libs {
        let p = st.ext.progress(id);
        looked += p.looked_up;
        found += p.found;
        rated += p.rated;
        if let Ok((_, cat)) = st.catalog(id) {
            total += cat.stats().live_book_count;
        }
    }
    json!({
        "enabled": cfg.enabled,
        "source": "openlibrary",
        "contactSet": st.cfg.contact_email.is_some(),
        "progress": { "lookedUp": looked, "found": found, "rated": rated, "total": total },
        "queued": st.ext.queued(),
        "requests": st.ext.requests.load(std::sync::atomic::Ordering::Relaxed),
        "pausedFor": st.ext.paused_for(),
        "lastError": st.ext.last_error(),
    })
}

fn settings_json(
    st: &AppState,
    smtp: &SmtpConfig,
    opds: &OpdsConfig,
    ext: &ExtRatingsConfig,
    mcp: &McpConfig,
) -> Value {
    json!({
        "externalRatings": ext_status(st, ext),
        "mcp": { "enabled": mcp.enabled, "url": st.cfg.public_url.as_ref().map(|u| format!("{u}/mcp")) },
        "smtp": {
            "host": smtp.host, "port": smtp.port, "security": smtp.security, "username": smtp.username,
            "from": smtp.from, "passwordSet": smtp.password.as_deref().is_some_and(|p| !p.is_empty()),
            "pauseSeconds": smtp.pause_seconds,
            "allowedRecipients": smtp.allowed_recipients,
            "dailyLimitPerUser": smtp.daily_limit_per_user,
            "subject": smtp.subject,
            "maxAttachments": smtp.max_attachments,
            "maxMailMb": smtp.max_mail_mb,
            "retries": smtp.retries,
            "retryDelaySeconds": smtp.retry_delay_seconds,
        },
        "opds": opds,
        "calibre": {
            "available": st.calibre.is_some(),
            "version": st.calibre.as_ref().and_then(|c| c.version.clone()),
        },
    })
}

type AllSettings = (SmtpConfig, OpdsConfig, ExtRatingsConfig, McpConfig);

fn load_all(c: &rusqlite::Connection) -> ApiResult<AllSettings> {
    Ok((
        db::get_setting::<SmtpConfig>(c, "smtp")?,
        db::get_setting::<OpdsConfig>(c, "opds")?,
        db::get_setting::<ExtRatingsConfig>(c, "externalRatings")?,
        db::get_setting::<McpConfig>(c, "mcp")?,
    ))
}

pub async fn get(State(st): State<AppState>, Admin(_): Admin) -> ApiResult<Json<Value>> {
    let (smtp, opds, ext, mcp) = st.db.run(|c| load_all(c)).await?;
    Ok(Json(settings_json(&st, &smtp, &opds, &ext, &mcp)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmtpIn {
    host: Option<String>,
    port: Option<u16>,
    security: Option<String>,
    username: Option<String>,
    from: Option<String>,
    password: Option<String>,
    pause_seconds: Option<u64>,
    allowed_recipients: Option<Vec<String>>,
    daily_limit_per_user: Option<u32>,
    subject: Option<String>,
    max_attachments: Option<u32>,
    max_mail_mb: Option<u32>,
    retries: Option<u32>,
    retry_delay_seconds: Option<u64>,
}

/// Recipient patterns: at most 100, each `*` or containing `@`, no spaces or control characters.
fn clean_patterns(v: Vec<String>) -> ApiResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for p in v {
        let p = p.trim().to_lowercase();
        if p.is_empty() {
            continue;
        }
        if p.len() > 254
            || (p != "*" && !p.contains('@'))
            || p.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(ApiError::bad_request(format!(
                "invalid recipient pattern '{p}' (use e.g. *@kindle.com)"
            )));
        }
        if !out.contains(&p) {
            out.push(p);
        }
    }
    if out.len() > 100 {
        return Err(ApiError::bad_request("at most 100 recipient patterns"));
    }
    Ok(out)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpdsIn {
    enabled: Option<bool>,
    require_auth: Option<bool>,
}

#[derive(Deserialize)]
pub struct EnabledIn {
    enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsIn {
    smtp: Option<SmtpIn>,
    opds: Option<OpdsIn>,
    external_ratings: Option<EnabledIn>,
    mcp: Option<EnabledIn>,
}

pub async fn put(
    State(st): State<AppState>,
    Admin(_): Admin,
    Json(b): Json<SettingsIn>,
) -> ApiResult<Json<Value>> {
    if let Some(s) = &b.smtp
        && let Some(sec) = &s.security
        && !["none", "starttls", "tls"].contains(&sec.as_str())
    {
        return Err(ApiError::bad_request(
            "security must be none, starttls or tls",
        ));
    }
    let patterns = match b.smtp.as_ref().and_then(|s| s.allowed_recipients.clone()) {
        Some(v) => Some(clean_patterns(v)?),
        None => None,
    };
    let (smtp, opds, ext, mcp) = st
        .db
        .run(move |c| {
            let (_, _, mut ext, mut mcp) = load_all(c)?;
            if let Some(v) = b.external_ratings.and_then(|e| e.enabled) {
                ext.enabled = v;
                db::put_setting(c, "externalRatings", &ext)?;
            }
            if let Some(v) = b.mcp.and_then(|e| e.enabled) {
                mcp.enabled = v;
                db::put_setting(c, "mcp", &mcp)?;
            }
            let mut smtp: SmtpConfig = db::get_setting(c, "smtp")?;
            if let Some(p) = patterns {
                smtp.allowed_recipients = p;
            }
            let mut opds: OpdsConfig = db::get_setting(c, "opds")?;
            if let Some(s) = b.smtp {
                if let Some(v) = s.host {
                    smtp.host = v.trim().to_string();
                }
                if let Some(v) = s.port {
                    smtp.port = v;
                }
                if let Some(v) = s.security {
                    smtp.security = v;
                }
                if let Some(v) = s.username {
                    smtp.username = v.trim().to_string();
                }
                if let Some(v) = s.from {
                    smtp.from = v.trim().to_string();
                }
                if let Some(v) = s.password {
                    // write-only; an empty string clears it
                    smtp.password = (!v.is_empty()).then_some(v);
                }
                if let Some(v) = s.pause_seconds {
                    smtp.pause_seconds = v.min(600);
                }
                if let Some(v) = s.daily_limit_per_user {
                    smtp.daily_limit_per_user = v.min(100_000);
                }
                if let Some(v) = s.max_attachments {
                    smtp.max_attachments = v.clamp(1, 100);
                }
                if let Some(v) = s.max_mail_mb {
                    smtp.max_mail_mb = v.clamp(1, 200);
                }
                if let Some(v) = s.retries {
                    smtp.retries = v.min(10);
                }
                if let Some(v) = s.retry_delay_seconds {
                    smtp.retry_delay_seconds = v.min(3600);
                }
                if let Some(v) = s.subject {
                    smtp.subject = v
                        .trim()
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(250)
                        .collect();
                }
            }
            if let Some(o) = b.opds {
                if let Some(v) = o.enabled {
                    opds.enabled = v;
                }
                if let Some(v) = o.require_auth {
                    opds.require_auth = v;
                }
            }
            db::put_setting(c, "smtp", &smtp)?;
            db::put_setting(c, "opds", &opds)?;
            Ok((smtp, opds, ext, mcp))
        })
        .await?;
    st.ext.set_enabled(ext.enabled);
    Ok(Json(settings_json(&st, &smtp, &opds, &ext, &mcp)))
}

#[derive(Deserialize)]
pub struct SmtpTest {
    to: String,
}

pub async fn smtp_test(
    State(st): State<AppState>,
    Admin(_): Admin,
    Json(b): Json<SmtpTest>,
) -> ApiResult<StatusCode> {
    let smtp: SmtpConfig = st.db.run(|c| db::get_setting(c, "smtp")).await?;
    crate::mail::send(
        &smtp,
        &b.to,
        "freeLib test message",
        "SMTP settings of freeLib work.",
        None,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Users with how they sign in: `hasPassword`, and `sso` (the linked single sign-on identity:
/// `email`, `createdAt`, `lastLogin`) or `null`.
pub async fn users(State(st): State<AppState>, Admin(_): Admin) -> ApiResult<Json<Vec<Value>>> {
    let issuer = st.oidc.as_ref().map(|p| p.issuer().to_string());
    let rows = st
        .db
        .run(move |c| {
            let users = db::list_users(c)?;
            let ids = db::identities(c)?;
            let mut out = Vec::with_capacity(users.len());
            for u in users {
                let hp = db::has_password(c, u.id)?;
                let sso = ids
                    .get(&u.id)
                    .filter(|i| issuer.as_deref().is_none_or(|x| x == i.issuer));
                out.push(json!({
                    "id": u.id,
                    "username": u.username,
                    "role": u.role,
                    "hasPassword": hp,
                    "sso": sso,
                }));
            }
            Ok(out)
        })
        .await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct NewUser {
    username: String,
    password: String,
    role: Option<String>,
}

fn valid_role(r: &str) -> ApiResult<()> {
    if r == "admin" || r == "reader" {
        Ok(())
    } else {
        Err(ApiError::bad_request("role must be admin or reader"))
    }
}

pub async fn create_user(
    State(st): State<AppState>,
    Admin(_): Admin,
    Json(b): Json<NewUser>,
) -> ApiResult<Json<User>> {
    auth::validate_username(&b.username)?;
    auth::validate_password(&b.password)?;
    let role = b.role.unwrap_or_else(|| "reader".into());
    valid_role(&role)?;
    let fast = st.cfg.fast_password_hash;
    let pw = b.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&pw, fast)).await??;
    let name = b.username;
    let user = st
        .db
        .run(move |c| db::insert_user(c, &name, &hash, &role))
        .await?;
    if st.open_mode() {
        // the first account ends open mode only once an administrator exists
        let admins = st.db.run(|c| db::count_admins(c)).await?;
        if admins > 0 {
            st.open_mode
                .store(false, std::sync::atomic::Ordering::Relaxed);
            tracing::info!("first administrator created: open mode off");
        }
    }
    Ok(Json(user))
}

#[derive(Deserialize)]
pub struct PatchUser {
    password: Option<String>,
    role: Option<String>,
}

pub async fn update_user(
    State(st): State<AppState>,
    Admin(me): Admin,
    Path(id): Path<i64>,
    Json(b): Json<PatchUser>,
) -> ApiResult<Json<User>> {
    if let Some(r) = &b.role {
        valid_role(r)?;
    }
    let hash = match b.password {
        Some(pw) => {
            auth::validate_password(&pw)?;
            let fast = st.cfg.fast_password_hash;
            Some(tokio::task::spawn_blocking(move || auth::hash_password(&pw, fast)).await??)
        }
        None => None,
    };
    let role = b.role;
    let user = st
        .db
        .run(move |c| {
            let u = db::get_user(c, id)?.ok_or_else(|| ApiError::not_found("user not found"))?;
            if role.as_deref() == Some("reader") && u.is_admin() && db::count_admins(c)? <= 1 {
                return Err(ApiError::conflict(
                    "the last administrator cannot be demoted",
                ));
            }
            db::update_user(c, id, hash.as_deref(), role.as_deref())?;
            db::get_user(c, id)?.ok_or_else(|| ApiError::not_found("user not found"))
        })
        .await?;
    st.invalidate_sessions();
    let _ = st.events().send(crate::jobs::Event::Users);
    let _ = me;
    Ok(Json(user))
}

pub async fn delete_user(
    State(st): State<AppState>,
    Admin(me): Admin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    if id == me.id {
        return Err(ApiError::conflict("you cannot delete yourself"));
    }
    st.db
        .run(move |c| {
            let u = db::get_user(c, id)?.ok_or_else(|| ApiError::not_found("user not found"))?;
            if u.is_admin() && db::count_admins(c)? <= 1 {
                return Err(ApiError::conflict(
                    "the last administrator cannot be deleted",
                ));
            }
            db::delete_user(c, id)?;
            Ok(())
        })
        .await?;
    st.invalidate_sessions();
    // its jobs (and their files) go too; ids are never reused, see db::insert_user
    for d in st.jobs.purge_user(id) {
        let _ = tokio::fs::remove_dir_all(d).await;
    }
    let _ = st.events().send(crate::jobs::Event::Users);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_prefs(State(st): State<AppState>, Auth(u): Auth) -> ApiResult<Json<Value>> {
    let s = st.db.run(move |c| db::get_prefs(c, u.id)).await?;
    Ok(Json(serde_json::from_str(&s).unwrap_or_else(|_| json!({}))))
}

pub async fn put_prefs(
    State(st): State<AppState>,
    Auth(u): Auth,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    if body.len() > PREFS_LIMIT {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "bad_request",
            "prefs must be at most 64 KB",
        ));
    }
    let v: Value = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid JSON: {e}")))?;
    let s = serde_json::to_string(&v).map_err(|e| ApiError::internal(e.to_string()))?;
    st.db.run(move |c| db::set_prefs(c, u.id, &s)).await?;
    Ok(Json(v))
}
