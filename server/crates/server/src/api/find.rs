//! Start page, follows and editions (`docs/web/API.md` "Start page and follows").

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::find;
use crate::state::AppState;
use crate::util::json_etag;

#[derive(Deserialize)]
pub struct HomeQuery {
    /// "New" window in days; absent = since the previous visit.
    days: Option<String>,
}

/// `GET /libraries/:lib/home`
pub async fn home(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(lib): Path<i64>,
    Query(q): Query<HomeQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let days = match q.days.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        None | Some("visit") => None,
        Some(d) => Some(
            d.parse::<i64>()
                .ok()
                .filter(|d| (1..=3650).contains(d))
                .ok_or_else(|| ApiError::bad_request("days must be 1..3650 or \"visit\""))?,
        ),
    };
    let st2 = st.clone();
    let v = st
        .catalog_call(lib, move |cat| {
            let h = find::home(&st2, u.id, lib, cat, days)?;
            serde_json::to_value(h).map_err(|e| ApiError::internal(e.to_string()))
        })
        .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct DismissBody {
    series: i64,
    #[serde(default = "yes")]
    dismissed: bool,
}

fn yes() -> bool {
    true
}

/// `POST /libraries/:lib/home/dismiss`
pub async fn dismiss(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(lib): Path<i64>,
    Json(b): Json<DismissBody>,
) -> ApiResult<Response> {
    let st2 = st.clone();
    st.catalog_call(lib, move |cat| {
        find::dismiss(&st2, u.id, lib, cat, b.series, b.dismissed)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /libraries/:lib/follows`
pub async fn follows(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(lib): Path<i64>,
) -> ApiResult<Json<find::FollowList>> {
    let st2 = st.clone();
    Ok(Json(
        st.catalog_call(lib, move |cat| find::follow_list(&st2, u.id, lib, cat))
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct FollowBody {
    kind: String,
    id: i64,
    follow: bool,
}

/// `PUT /libraries/:lib/follows`
pub async fn set_follow(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(lib): Path<i64>,
    Json(b): Json<FollowBody>,
) -> ApiResult<Json<find::FollowList>> {
    let st2 = st.clone();
    Ok(Json(
        st.catalog_call(lib, move |cat| {
            find::follow(&st2, u.id, lib, cat, &b.kind, b.id, b.follow)
        })
        .await?,
    ))
}

/// `GET /libraries/:lib/books/:id/editions`
pub async fn editions(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path((lib, id)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let st2 = st.clone();
    let v = st
        .catalog_call(lib, move |cat| find::editions(&st2, u.id, lib, cat, id))
        .await?;
    json_etag(&headers, &v)
}
