use axum::Extension;
use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;

use crate::auth::{self, COOKIE};
use crate::db::{self, User};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::{random_token, set_header, unix_now};

/// A new visit starts after this long without `GET /session`.
const VISIT_GAP_SECS: i64 = 30 * 60;

pub async fn get_session(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    let user = auth::current_user(&st, &headers).await?;
    if let Some(u) = &user {
        track_visit(&st, u.id).await?;
    }
    let oidc = st
        .oidc
        .as_ref()
        .map(|p| json!({ "enabled": true, "label": p.cfg.button }));
    let password = !st.cfg.oidc.as_ref().is_some_and(|o| o.disable_password);
    let mut r = Json(json!({
        "user": user,
        "openMode": st.open_mode(),
        "auth": { "password": password, "oidc": oidc },
    }))
    .into_response();
    set_header(&mut r, header::CACHE_CONTROL, "no-store");
    Ok(r)
}

/// Remembers the previous visit (for `newSinceLastVisit`, persisted in `user_state`) and
/// stamps the current one.
async fn track_visit(st: &AppState, user_id: i64) -> ApiResult<()> {
    st.db
        .run(move |c| db::track_visit(c, user_id, unix_now(), VISIT_GAP_SECS))
        .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct LoginBody {
    username: String,
    password: String,
}

pub async fn login(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(b): Json<LoginBody>,
) -> ApiResult<Response> {
    if st.open_mode() {
        return Ok(Json(json!({ "user": User::open_mode_admin() })).into_response());
    }
    if st.password_login_refused(&b.username) {
        return Err(ApiError::forbidden(
            "password sign-in is disabled: use single sign-on",
        ));
    }
    let ip = auth::client_ip(&headers, peer.map(|p| p.0.0), st.cfg.trust_proxy);
    let user = auth::check_credentials(&st, ip, b.username.trim(), &b.password).await?;
    let token = random_token(32);
    let (uid, t2) = (user.id, token.clone());
    // a fresh session id; a session this browser had before is dropped
    let old = auth::cookie_value(&headers, COOKIE);
    st.db
        .run(move |c| {
            if let Some(o) = old {
                db::delete_session(c, &o)?;
            }
            db::create_session(c, uid, &t2)
        })
        .await?;
    tracing::info!(user = %user.username, "login");
    let mut r = Json(json!({ "user": user })).into_response();
    set_header(
        &mut r,
        header::SET_COOKIE,
        &auth::session_cookie(&token, st.secure_cookies(&headers)),
    );
    set_header(&mut r, header::CACHE_CONTROL, "no-store");
    Ok(r)
}

pub async fn logout(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = auth::cookie_value(&headers, COOKIE) {
        st.db.run(move |c| db::delete_session(c, &token)).await?;
        st.invalidate_sessions();
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    set_header(&mut r, header::SET_COOKIE, &auth::clear_cookie());
    Ok(r)
}
