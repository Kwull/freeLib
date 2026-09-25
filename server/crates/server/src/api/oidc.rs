//! Single sign-on endpoints (`/auth/oidc/*`) and the account endpoints that go with it
//! (`/me/account`, `/me/password`, `/me/oidc`). The flow itself is in [`crate::oidc`].

use std::net::SocketAddr;

use axum::Extension;
use axum::Json;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{self, Auth, COOKIE};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::oidc::{
    self, CallbackQuery, Outcome, PENDING_TTL, STATE_COOKIE, STATE_COOKIE_PATH, SsoError,
};
use crate::state::{AppState, LimitKey};
use crate::util::{random_token, set_header};

fn state_cookie(value: &str, max_age: u64, secure: bool) -> String {
    format!(
        "{STATE_COOKIE}={value}; Path={STATE_COOKIE_PATH}; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    )
}

/// 303 to a same-origin path.
fn see_other(location: &str) -> Response {
    let mut r = StatusCode::SEE_OTHER.into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    set_header(&mut r, header::CACHE_CONTROL, "no-store");
    r
}

fn add_cookie(r: &mut Response, cookie: &str) {
    if let Ok(v) = HeaderValue::from_str(cookie) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
}

/// Where a failed sign-in lands: the login page, or Settings → Account for the link flow.
fn error_redirect(e: &SsoError, link: bool) -> Response {
    let page = if link { "/settings/account" } else { "/login" };
    see_other(&format!("{page}?ssoError={}", e.code))
}

/// The per-client key of pending sign-ins: the address (IPv6 per /64, like the login limiter).
fn client_key(ip: std::net::IpAddr) -> String {
    format!("{:?}", LimitKey::ip(ip))
}

fn provider(st: &AppState) -> ApiResult<std::sync::Arc<oidc::Provider>> {
    st.oidc
        .clone()
        .ok_or_else(|| ApiError::not_found("single sign-on is not configured"))
}

#[derive(Deserialize)]
pub struct LoginQuery {
    #[serde(rename = "return")]
    return_to: Option<String>,
}

/// `GET /auth/oidc/login?return=/path`: redirects to the provider.
pub async fn login(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Query(q): Query<LoginQuery>,
) -> ApiResult<Response> {
    let p = provider(&st)?;
    let ret = oidc::safe_return(q.return_to.as_deref());
    let ip = auth::client_ip(&headers, peer.map(|p| p.0.0), st.cfg.trust_proxy);
    match p.begin(ret, None, client_key(ip)).await {
        Ok((url, state)) => {
            let mut r = StatusCode::SEE_OTHER.into_response();
            let loc = HeaderValue::from_str(&url)
                .map_err(|_| ApiError::internal("bad authorization URL"))?;
            r.headers_mut().insert(header::LOCATION, loc);
            set_header(&mut r, header::CACHE_CONTROL, "no-store");
            add_cookie(
                &mut r,
                &state_cookie(&state, PENDING_TTL.as_secs(), st.secure_cookies(&headers)),
            );
            Ok(r)
        }
        Err(e) => {
            tracing::warn!("single sign-on could not start: {e}");
            Ok(error_redirect(&e, false))
        }
    }
}

/// `POST /auth/oidc/link`: starts linking the provider identity to the current user;
/// answers `{ "url": <authorization URL> }` for the web app to navigate to.
pub async fn link(
    State(st): State<AppState>,
    Auth(u): Auth,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let p = provider(&st)?;
    if st.open_mode() || u.id == 0 {
        return Err(ApiError::bad_request("no account to link in open mode"));
    }
    let (url, state) = p
        .begin(
            "/settings/account".into(),
            Some(u.id),
            format!("user:{}", u.id),
        )
        .await
        .map_err(|e| {
            tracing::warn!("single sign-on could not start: {e}");
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "internal",
                "the sign-in provider cannot be reached",
            )
        })?;
    let mut r = Json(json!({ "url": url })).into_response();
    add_cookie(
        &mut r,
        &state_cookie(&state, PENDING_TTL.as_secs(), st.secure_cookies(&headers)),
    );
    set_header(&mut r, header::CACHE_CONTROL, "no-store");
    Ok(r)
}

/// `GET /auth/oidc/callback`: finishes the sign-in and redirects into the web app.
pub async fn callback(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> ApiResult<Response> {
    let p = provider(&st)?;
    let secure = st.secure_cookies(&headers);
    let clear_state = state_cookie("", 0, secure);
    let link_flow = p.peek_link(q.state.as_deref());
    let ip = auth::client_ip(&headers, peer.map(|p| p.0.0), st.cfg.trust_proxy);
    let keys = [LimitKey::ip(ip)];
    if st.login_limiter.begin(&keys).is_err() {
        let mut r = error_redirect(
            &SsoError {
                code: "rate_limited",
                detail: String::new(),
            },
            link_flow,
        );
        add_cookie(&mut r, &clear_state);
        return Ok(r);
    }
    let cookie = auth::cookie_value(&headers, STATE_COOKIE);
    let result = async {
        let v = p.finish(&q, cookie.as_deref()).await?;
        if let Some(uid) = v.link_user {
            // the account being linked must still be the one signed in in this browser
            match auth::current_user(&st, &headers).await {
                Ok(Some(cur)) if cur.id == uid => {}
                _ => {
                    return Err(SsoError {
                        code: "session",
                        detail: "not signed in as the account being linked".into(),
                    });
                }
            }
        }
        let (st2, cfg) = (st.clone(), p.cfg.clone());
        let ret = v.return_to.clone();
        let link = v.link_user.is_some();
        let outcome = tokio::task::spawn_blocking(move || oidc::resolve_account(&st2, &cfg, &v))
            .await
            .map_err(|e| SsoError {
                code: "internal",
                detail: e.to_string(),
            })??;
        Ok((outcome, ret, link))
    }
    .await;
    st.login_limiter.finish(&keys, result.is_ok());
    let (outcome, ret, link) = match result {
        Ok(x) => x,
        Err(e) => {
            tracing::info!(%ip, "single sign-on failed: {e}");
            let mut r = error_redirect(&e, link_flow);
            add_cookie(&mut r, &clear_state);
            return Ok(r);
        }
    };
    let user = match &outcome {
        Outcome::SignedIn(u) | Outcome::Linked(u) => u.clone(),
    };
    // a fresh session id for every sign-in (and link); the old one is dropped
    let old = auth::cookie_value(&headers, COOKIE);
    let token = random_token(32);
    let (uid, t2) = (user.id, token.clone());
    st.db
        .run(move |c| {
            if let Some(o) = old {
                db::delete_session(c, &o)?;
            }
            db::create_session(c, uid, &t2)
        })
        .await?;
    st.invalidate_sessions();
    let target = if link {
        "/settings/account?sso=linked".to_string()
    } else {
        tracing::info!(user = %user.username, "login (single sign-on)");
        ret
    };
    let mut r = see_other(&target);
    add_cookie(&mut r, &auth::session_cookie(&token, secure));
    add_cookie(&mut r, &clear_state);
    Ok(r)
}

/// `GET /me/account`: how the current user can sign in.
pub async fn account(
    State(st): State<AppState>,
    Auth(u): Auth,
) -> ApiResult<Json<serde_json::Value>> {
    let issuer = st.oidc.as_ref().map(|p| p.issuer().to_string());
    let uid = u.id;
    let (has_password, ident) = st
        .db
        .run(move |c| {
            let hp = db::has_password(c, uid)?;
            let id = match &issuer {
                Some(i) => db::user_identity(c, uid, i)?,
                None => None,
            };
            Ok((hp, id))
        })
        .await?;
    let sso = st.oidc.as_ref().map(|p| {
        json!({
            "label": p.cfg.button,
            "linked": ident.is_some(),
            "email": ident.as_ref().and_then(|i| i.email.clone()),
            "lastLogin": ident.as_ref().and_then(|i| i.last_login.clone()),
        })
    });
    Ok(Json(json!({
        "user": u,
        "hasPassword": has_password || st.open_mode(),
        "passwordLogin": !st.password_login_refused(&u.username),
        "sso": sso,
    })))
}

#[derive(Deserialize)]
pub struct PasswordBody {
    current: Option<String>,
    password: String,
}

/// `PUT /me/password`: sets the user's own password (the current one is required when there
/// is one). Signs out the user's other sessions; this one gets a new session id.
pub async fn set_password(
    State(st): State<AppState>,
    Auth(u): Auth,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(b): Json<PasswordBody>,
) -> ApiResult<Response> {
    if st.open_mode() || u.id == 0 {
        return Err(ApiError::bad_request("there are no accounts in open mode"));
    }
    auth::validate_password(&b.password)?;
    let uid = u.id;
    let had = st.db.run(move |c| db::has_password(c, uid)).await?;
    if had {
        let cur = b.current.unwrap_or_default();
        let ip = auth::client_ip(&headers, peer.map(|p| p.0.0), st.cfg.trust_proxy);
        let who = auth::check_credentials(&st, ip, &u.username, &cur)
            .await
            .map_err(|e| {
                if e.code == "unauthorized" {
                    ApiError::forbidden("the current password is wrong")
                } else {
                    e
                }
            })?;
        if who.id != u.id {
            return Err(ApiError::forbidden("the current password is wrong"));
        }
    }
    let fast = st.cfg.fast_password_hash;
    let pw = b.password;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&pw, fast)).await??;
    let token = random_token(32);
    let t2 = token.clone();
    st.db
        .run(move |c| {
            db::update_user(c, uid, Some(&hash), None)?; // drops all sessions
            db::create_session(c, uid, &t2)
        })
        .await?;
    st.invalidate_sessions();
    tracing::info!(user = %u.username, "password changed");
    let mut r = StatusCode::NO_CONTENT.into_response();
    set_header(
        &mut r,
        header::SET_COOKIE,
        &auth::session_cookie(&token, st.secure_cookies(&headers)),
    );
    Ok(r)
}

/// `DELETE /me/oidc`: unlinks the user's single sign-on identity. Refused while it is the
/// only way the user can sign in.
pub async fn unlink(State(st): State<AppState>, Auth(u): Auth) -> ApiResult<StatusCode> {
    provider(&st)?;
    if st.password_login_refused(&u.username) {
        return Err(ApiError::conflict(
            "password sign-in is disabled: single sign-on is the only way to sign in",
        ));
    }
    let uid = u.id;
    st.db
        .run(move |c| {
            if !db::has_password(c, uid)? {
                return Err(ApiError::conflict(
                    "set a password first, or you could not sign in any more",
                ));
            }
            if !db::unlink_identity(c, uid)? {
                return Err(ApiError::not_found("no single sign-on identity is linked"));
            }
            Ok(())
        })
        .await?;
    tracing::info!(user = %u.username, "single sign-on identity unlinked");
    Ok(StatusCode::NO_CONTENT)
}
