use std::convert::Infallible;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;

use crate::api::books::file_response;
use crate::auth::{self, Auth};
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
///
/// The user is re-checked (through the session cache, which user changes clear) before every
/// event: the stream ends when the session is gone, the user was deleted or an admin was
/// demoted; a role change re-evaluates which jobs are visible. The library DTO of a `library`
/// event is built once and shared by all subscribers.
pub async fn events(
    State(st): State<AppState>,
    Auth(u): Auth,
    headers: HeaderMap,
) -> impl IntoResponse {
    let rx = BroadcastStream::new(st.events().subscribe());
    let stream = futures_util::stream::unfold(
        (rx, st.clone(), headers, u),
        |(mut rx, st, headers, mut u)| async move {
            loop {
                let Ok(ev) = rx.next().await? else {
                    continue; // lagged receivers skip missed events
                };
                match auth::current_user(&st, &headers).await {
                    Ok(Some(cur)) if cur.id == u.id => {
                        if u.is_admin() && !cur.is_admin() {
                            return None;
                        }
                        u = cur;
                    }
                    _ => return None,
                }
                if !ev.visible_to(&u) {
                    continue;
                }
                let sse = match ev {
                    Event::Job { job, .. } => SseEvent::default().event("job").json_data(&job),
                    Event::Library { id, dto } => {
                        let st2 = st.clone();
                        let base = dto
                            .get_or_init(|| async move {
                                tokio::task::spawn_blocking(move || {
                                    st2.library_base_dto_by_id(id).ok()
                                })
                                .await
                                .ok()
                                .flatten()
                            })
                            .await
                            .clone();
                        let Some(mut lib) = base else { continue };
                        lib.new_since_last_visit = match st.new_since_cached(id, u.id) {
                            Some(n) => n,
                            None => {
                                let (st3, uid) = (st.clone(), u.id);
                                tokio::task::spawn_blocking(move || {
                                    st3.new_since_last_visit(id, uid)
                                })
                                .await
                                .unwrap_or(0)
                            }
                        };
                        SseEvent::default().event("library").json_data(&lib)
                    }
                    Event::Users => continue,
                };
                let Ok(sse) = sse else { continue };
                return Some((Ok::<SseEvent, Infallible>(sse), (rx, st, headers, u)));
            }
        },
    );
    (
        [
            (header::CACHE_CONTROL, "no-cache"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Sse::new(stream).keep_alive(KeepAlive::new().interval(st.cfg.sse_heartbeat).text("ping")),
    )
}
