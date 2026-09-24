use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{self, Admin, Auth};
use crate::db::{self, OpdsConfig, SmtpConfig, User};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const PREFS_LIMIT: usize = 64 * 1024;

fn settings_json(st: &AppState, smtp: &SmtpConfig, opds: &OpdsConfig) -> Value {
    json!({
        "smtp": {
            "host": smtp.host, "port": smtp.port, "security": smtp.security, "username": smtp.username,
            "from": smtp.from, "passwordSet": smtp.password.as_deref().is_some_and(|p| !p.is_empty()),
            "pauseSeconds": smtp.pause_seconds,
        },
        "opds": opds,
        "calibre": {
            "available": st.calibre.is_some(),
            "version": st.calibre.as_ref().and_then(|c| c.version.clone()),
        },
    })
}

pub async fn get(State(st): State<AppState>, Admin(_): Admin) -> ApiResult<Json<Value>> {
    let (smtp, opds) = st
        .db
        .run(|c| {
            Ok((
                db::get_setting::<SmtpConfig>(c, "smtp")?,
                db::get_setting::<OpdsConfig>(c, "opds")?,
            ))
        })
        .await?;
    Ok(Json(settings_json(&st, &smtp, &opds)))
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpdsIn {
    enabled: Option<bool>,
    require_auth: Option<bool>,
}

#[derive(Deserialize)]
pub struct SettingsIn {
    smtp: Option<SmtpIn>,
    opds: Option<OpdsIn>,
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
    let (smtp, opds) = st
        .db
        .run(move |c| {
            let mut smtp: SmtpConfig = db::get_setting(c, "smtp")?;
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
            Ok((smtp, opds))
        })
        .await?;
    Ok(Json(settings_json(&st, &smtp, &opds)))
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

pub async fn users(State(st): State<AppState>, Admin(_): Admin) -> ApiResult<Json<Vec<User>>> {
    Ok(Json(st.db.run(|c| db::list_users(c)).await?))
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
