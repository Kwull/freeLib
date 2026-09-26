//! `/me/tokens`: personal API tokens for the MCP endpoint (signed-in users, session cookie).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::Auth;
use crate::db::{self, McpConfig};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::tokens;

/// The MCP endpoint URL: `FREELIB_PUBLIC_URL` + `/mcp`, else from the request's host.
fn mcp_url(st: &AppState, headers: &HeaderMap) -> String {
    if let Some(u) = &st.cfg.public_url {
        return format!("{u}/mcp");
    }
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(axum::http::header::HOST))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .unwrap_or("localhost:8080");
    let scheme = if crate::auth::is_https(headers) {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://{host}/mcp")
}

/// `GET /me/tokens`: `{ tokens, scopes, mcp: {enabled, url, oauth} }` (`oauth`: whether apps
/// can connect by signing in, i.e. `FREELIB_PUBLIC_URL` is an https URL).
pub async fn list(
    State(st): State<AppState>,
    Auth(u): Auth,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let (list, mcp) = st
        .db
        .run(move |c| {
            Ok((
                db::list_tokens(c, u.id)?,
                db::get_setting::<McpConfig>(c, "mcp")?,
            ))
        })
        .await?;
    Ok(Json(json!({
        "tokens": list,
        "scopes": tokens::SCOPES,
        "mcp": {
            "enabled": mcp.enabled,
            "url": mcp_url(&st, &headers),
            "oauth": crate::oauth::issuer(&st).is_some(),
        },
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewToken {
    name: String,
    scopes: Vec<String>,
    /// Days until the token expires; `None` = never.
    expires_in_days: Option<u32>,
}

/// `POST /me/tokens`: creates a token; the secret is in the response only.
pub async fn create(
    State(st): State<AppState>,
    Auth(u): Auth,
    Json(b): Json<NewToken>,
) -> ApiResult<Json<Value>> {
    let name = b.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(|c| c.is_control()) {
        return Err(ApiError::bad_request(
            "token name must have 1..100 characters",
        ));
    }
    let scopes = tokens::clean_scopes(&b.scopes)?;
    let expires = match b.expires_in_days {
        None | Some(0) => None,
        Some(d) if d <= 3650 => Some(crate::util::rfc3339_at(
            crate::util::unix_now() + i64::from(d) * 86_400,
        )),
        Some(_) => return Err(ApiError::bad_request("expiresInDays must be at most 3650")),
    };
    let secret = tokens::generate();
    let (hash, prefix) = (tokens::hash(&secret), tokens::display_prefix(&secret));
    let tok = st
        .db
        .run(move |c| db::insert_token(c, u.id, &name, &hash, &prefix, &scopes, expires.as_deref()))
        .await?;
    Ok(Json(json!({ "token": tok, "secret": secret })))
}

/// `DELETE /me/tokens/:id`: revokes a token at once.
pub async fn revoke(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let gone = st.db.run(move |c| db::delete_token(c, u.id, id)).await?;
    st.tokens.invalidate();
    if !gone {
        return Err(ApiError::not_found("token not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /me/tokens/audit`: the last 50 MCP tool calls made with the user's tokens.
pub async fn audit(State(st): State<AppState>, Auth(u): Auth) -> ApiResult<Json<Value>> {
    let rows = st.db.run(move |c| db::audit(c, u.id, 50)).await?;
    Ok(Json(json!(rows)))
}
