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
use crate::error::ApiResult;
use crate::state::AppState;
use crate::util::{now_rfc3339, random_token, rfc3339_at, set_header, unix_now};

/// A new visit starts after this long without `GET /session`.
const VISIT_GAP_SECS: i64 = 30 * 60;

pub async fn get_session(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    let user = auth::current_user(&st, &headers).await?;
    if let Some(u) = &user {
        track_visit(&st, u.id).await?;
    }
    let mut r = Json(json!({ "user": user, "openMode": st.open_mode() })).into_response();
    set_header(&mut r, header::CACHE_CONTROL, "no-store");
    Ok(r)
}

/// Remembers the previous visit (for `newSinceLastVisit`) and stamps the current one.
async fn track_visit(st: &AppState, user_id: i64) -> ApiResult<()> {
    let st2 = st.clone();
    st.db
        .run(move |c| {
            let last = db::last_visit(c, user_id)?;
            let now = unix_now();
            let threshold = rfc3339_at(now - VISIT_GAP_SECS);
            let mut prev = st2.prev_visit.lock().unwrap_or_else(|e| e.into_inner());
            match &last {
                Some(l) if *l >= threshold => {
                    prev.entry(user_id).or_insert_with(|| l.clone());
                    // refresh at most every 5 minutes
                    if *l < rfc3339_at(now - 300) {
                        db::set_last_visit(c, user_id, &now_rfc3339())?;
                    }
                }
                Some(l) => {
                    prev.insert(user_id, l.clone());
                    db::set_last_visit(c, user_id, &now_rfc3339())?;
                }
                None => {
                    prev.insert(user_id, now_rfc3339());
                    db::set_last_visit(c, user_id, &now_rfc3339())?;
                }
            }
            Ok(())
        })
        .await
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
    let ip = auth::client_ip(&headers, peer.map(|p| p.0.0), st.cfg.trust_proxy);
    let user = auth::check_credentials(&st, ip, b.username.trim(), &b.password).await?;
    let token = random_token(32);
    let (uid, t2) = (user.id, token.clone());
    st.db.run(move |c| db::create_session(c, uid, &t2)).await?;
    tracing::info!(user = %user.username, "login");
    let mut r = Json(json!({ "user": user })).into_response();
    set_header(
        &mut r,
        header::SET_COOKIE,
        &auth::session_cookie(&token, auth::is_https(&headers)),
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
