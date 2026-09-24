//! freeLib web: catalog database (`lib_<id>.db`) schema and read API, `app.db` schema,
//! sort-key normalisation and the static genre table.
//!
//! The read API is synchronous (rusqlite); the HTTP server calls it from `spawn_blocking`.
//! See `README.md` for an overview.

pub mod catalog;
pub mod genres;
pub mod model;
pub mod normalize;
pub mod schema;
pub mod search;
pub mod util;

pub use catalog::{open_read_only, BookFilter, BookSelector, Catalog, CatalogError, CatalogHandle, Page, PooledConn};
pub use genres::{genres, GenreDef, Genres, GENRE_OTHER};
pub use model::*;
pub use normalize::{letter_of, normalize};
pub use schema::{
    create_catalog_indexes, create_catalog_tables, migrate_app_db, open_app_db, APP_SCHEMA_VERSION,
    CATALOG_SCHEMA_VERSION,
};
pub use search::{BookAttrs, SearchKind, SearchQuery};
