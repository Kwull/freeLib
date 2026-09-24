use std::convert::Infallible;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;

use crate::api::books::file_response;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::jobs::{Event, Job};
use crate::output::Produced;
use crate::state::AppState;

pub async fn list(State(st): State<AppState>, Auth(u): Auth) -> Json<Vec<Job>> {
    Json(st.jobs.list(&u))
}

pub async fn cancel(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Job>> {
    st.jobs
        .cancel(&id, &u)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("job not found"))
}

#[derive(Deserialize)]
pub struct ClearQuery {
    finished: Option<String>,
}

pub async fn clear(
    State(st): State<AppState>,
    Auth(u): Auth,
    Query(q): Query<ClearQuery>,
) -> ApiResult<StatusCode> {
    if q.finished.as_deref() != Some("1") {
        return Err(ApiError::bad_request("only finished=1 is supported"));
    }
    for d in st.jobs.clear_finished(&u) {
        let _ = tokio::fs::remove_dir_all(d).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn download(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let f = st
        .jobs
        .file(&id, &u)
        .ok_or_else(|| ApiError::not_found("no file for this job"))?;
    if !f.path.is_file() {
        return Err(ApiError::not_found("file expired"));
    }
    let ext = f.name.rsplit('.').next().unwrap_or("").to_string();
    file_response(Produced::File(f.path), &f.name, &ext, false).await
}

/// SSE stream; `X-Accel-Buffering: no` asks nginx-style proxies not to buffer it.
pub async fn events(State(st): State<AppState>, Auth(u): Auth) -> impl IntoResponse {
    let rx = st.events().subscribe();
    let st2 = st.clone();
    let stream = BroadcastStream::new(rx)
        .filter_map(move |ev| {
            let u = u.clone();
            let st = st2.clone();
            async move {
                let ev = ev.ok()?; // lagged receivers skip missed events
                if !ev.visible_to(&u) {
                    return None;
                }
                match ev {
                    Event::Job { job, .. } => {
                        Some(SseEvent::default().event("job").json_data(&job).ok()?)
                    }
                    Event::Library { id } => {
                        let lib = st.library_dto_async(id, u.id).await.ok()?;
                        Some(SseEvent::default().event("library").json_data(&lib).ok()?)
                    }
                }
            }
        })
        .map(Ok::<SseEvent, Infallible>);
    (
        [
            (header::CACHE_CONTROL, "no-cache"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Sse::new(stream).keep_alive(KeepAlive::new().interval(st.cfg.sse_heartbeat).text("ping")),
    )
}
