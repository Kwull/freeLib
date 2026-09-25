//! freeLib web edition server: HTTP API (`docs/web/API.md`), OPDS, auth, jobs, SMTP and the
//! embedded web app. See `README.md` for the module layout.

pub mod api;
pub mod app;
pub mod auth;
pub mod bookio;
pub mod cache;
pub mod calibre;
pub mod compress;
pub mod config;
pub mod conv;
pub mod db;
pub mod error;
pub mod importer;
pub mod jobs;
pub mod mail;
pub mod oidc;
pub mod opds;
pub mod output;
pub mod placeholder;
pub mod preview;
pub mod security;
pub mod sender;
pub mod spa;
pub mod state;
pub mod util;

pub use app::{init, router};
pub use config::Config;
pub use state::AppState;
