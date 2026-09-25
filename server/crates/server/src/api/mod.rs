//! `/api/v1` routes (docs/web/API.md).

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post, put};

use crate::state::AppState;

pub mod books;
pub mod browse;
pub mod devices;
pub mod jobs;
pub mod libraries;
pub mod oidc;
pub mod session;
pub mod settings;
pub mod shelves;

/// JSON request bodies above this size are refused.
pub const BODY_LIMIT: usize = 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/session", get(session::get_session))
        .route("/login", post(session::login))
        .route("/logout", post(session::logout))
        .route("/auth/oidc/login", get(oidc::login))
        .route("/auth/oidc/callback", get(oidc::callback))
        .route("/auth/oidc/link", post(oidc::link))
        .route("/me/account", get(oidc::account))
        .route("/me/password", put(oidc::set_password))
        .route("/me/oidc", axum::routing::delete(oidc::unlink))
        .route("/libraries", get(libraries::list).post(libraries::create))
        .route(
            "/libraries/{lib}",
            axum::routing::patch(libraries::update).delete(libraries::delete),
        )
        .route("/libraries/{lib}/import", post(libraries::import))
        .route("/fs", get(libraries::fs))
        .route("/libraries/{lib}/authors", get(browse::authors))
        .route(
            "/libraries/{lib}/authors/{id}/summary",
            get(browse::author_summary),
        )
        .route(
            "/libraries/{lib}/authors/{id}/coauthors",
            get(browse::coauthors),
        )
        .route("/libraries/{lib}/series", get(browse::series))
        .route("/libraries/{lib}/genres", get(browse::genres))
        .route("/libraries/{lib}/books", get(browse::books))
        .route("/libraries/{lib}/search", get(browse::search))
        .route("/languages", get(browse::languages))
        .route("/libraries/{lib}/books/{id}", get(books::detail))
        .route("/libraries/{lib}/books/{id}/cover", get(books::cover))
        .route("/libraries/{lib}/books/{id}/file", get(books::file))
        .route("/libraries/{lib}/books/{id}/rating", put(shelves::rating))
        .route("/shelves", get(shelves::list).post(shelves::create))
        .route(
            "/shelves/{id}",
            axum::routing::patch(shelves::update).delete(shelves::delete),
        )
        .route("/shelves/{id}/books", post(shelves::books))
        .route("/devices", get(devices::list).post(devices::create))
        .route(
            "/devices/{id}",
            put(devices::update).delete(devices::delete),
        )
        .route("/send", post(devices::send))
        .route("/fonts", get(devices::fonts))
        .route("/jobs", get(jobs::list).delete(jobs::clear))
        .route("/jobs/{id}/cancel", post(jobs::cancel))
        .route("/jobs/{id}/download", get(jobs::download))
        .route("/events", get(jobs::events))
        .route("/settings", get(settings::get).put(settings::put))
        .route("/settings/smtp/test", post(settings::smtp_test))
        .route("/users", get(settings::users).post(settings::create_user))
        .route(
            "/users/{id}",
            axum::routing::patch(settings::update_user).delete(settings::delete_user),
        )
        .route(
            "/me/prefs",
            get(settings::get_prefs).put(settings::put_prefs),
        )
        .fallback(|| async { crate::error::ApiError::not_found("no such endpoint") })
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
}

/// Parses a comma-separated list, ignoring empty items.
pub fn split_list(s: Option<&str>) -> Vec<String> {
    s.map(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(String::from)
            .collect()
    })
    .unwrap_or_default()
}
