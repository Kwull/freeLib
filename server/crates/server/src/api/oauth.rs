//! The web app's side of OAuth: the consent page (`/oauth/requests/{id}`) and the list of
//! authorized apps (`/me/oauth/apps`). The authorization server itself is [`crate::oauth`].

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::Auth;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::oauth::{self, Decision, DecisionError, store};
use crate::state::AppState;
use crate::util::set_header;

fn gone() -> ApiError {
    ApiError::not_found(
        "this authorization request is unknown or expired; start the connection again in the app",
    )
}

/// `GET /oauth/requests/{id}`: what the consent page shows (app, redirect host, scopes) and
/// the request's CSRF token.
pub async fn request(
    State(st): State<AppState>,
    Auth(_u): Auth,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let v = oauth::pending_view(&st, &id).ok_or_else(gone)?;
    let mut r = Json(v).into_response();
    set_header(&mut r, header::CACHE_CONTROL, "no-store");
    Ok(r)
}

#[derive(Deserialize)]
pub struct DecisionIn {
    approve: bool,
    #[serde(default)]
    scopes: Vec<String>,
    csrf: String,
}

/// `POST /oauth/requests/{id}`: approves (with the chosen scopes) or denies; answers
/// `{ "redirect": <URL> }` for the browser to go to.
pub async fn decide(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<String>,
    Json(b): Json<DecisionIn>,
) -> ApiResult<Json<Value>> {
    let d = if b.approve {
        Decision::Approve(b.scopes)
    } else {
        Decision::Deny
    };
    match oauth::decide(&st, &u, &id, &b.csrf, d).await {
        Ok(url) => Ok(Json(json!({ "redirect": url }))),
        Err(DecisionError::NotFound) => Err(gone()),
        Err(DecisionError::Csrf) => Err(ApiError::forbidden("invalid request token")),
        Err(DecisionError::Scopes(m)) => Err(ApiError::bad_request(m)),
    }
}

/// `GET /me/oauth/apps`: the user's authorized apps.
pub async fn apps(State(st): State<AppState>, Auth(u): Auth) -> ApiResult<Json<Value>> {
    let rows = st.db.run(move |c| store::list_grants(c, u.id)).await?;
    let out: Vec<Value> = rows
        .into_iter()
        .map(|g| {
            let redirect_host = oauth::Url::parse(&g.redirect_uri)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .unwrap_or_default();
            let verified_host = (g.client_kind == "cimd")
                .then(|| {
                    oauth::Url::parse(&g.client_id)
                        .ok()
                        .and_then(|u| u.host_str().map(String::from))
                })
                .flatten();
            json!({
                "id": g.id,
                "clientName": g.client_name,
                "clientKind": g.client_kind,
                "verifiedHost": verified_host,
                "redirectHost": redirect_host,
                "scopes": g.scopes,
                "createdAt": g.created_at,
                "lastUsedAt": g.last_used_at,
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

/// `DELETE /me/oauth/apps/{id}`: revokes an authorized app (all its tokens at once).
pub async fn revoke_app(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let uid = u.id;
    let name = st
        .db
        .run(move |c| {
            let n = store::revoke_user_grant(c, uid, id)?;
            if let Some(n) = &n {
                db::add_audit(
                    c,
                    uid,
                    None,
                    Some(id),
                    "oauth.revoke",
                    true,
                    &format!("{n}: access revoked in Settings"),
                )?;
            }
            Ok(n)
        })
        .await?;
    st.tokens.invalidate();
    match name {
        Some(n) => {
            tracing::info!(user = %u.username, app = %n, "OAuth app revoked");
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err(ApiError::not_found("app not found")),
    }
}
