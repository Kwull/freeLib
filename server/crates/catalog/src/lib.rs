//! freeLib web: catalog database (`lib_<id>.db`) schema and read API, `app.db` schema,
//! sort-key normalisation and the static genre table.
//!
//! The read API is synchronous (rusqlite); the HTTP server calls it from `spawn_blocking`.
//! See `README.md` for an overview.

pub mod catalog;
pub mod genres;
pub mod home;
pub mod kids;
pub mod model;
pub mod normalize;
pub mod rank;
pub mod schema;
pub mod search;
pub mod summary;
pub mod text;
pub mod util;
pub mod vocab;
pub mod works;

pub use catalog::{
    BookFilter, BookSelector, Catalog, CatalogError, CatalogHandle, Page, PooledConn,
    open_read_only,
};
pub use genres::{GENRE_OTHER, GenreDef, Genres, genres};
pub use model::*;
pub use normalize::{letter_of, normalize};
pub use rank::{NoRatings, RatingQuery, RatingSort, RatingSource};
pub use schema::{
    APP_SCHEMA_VERSION, CATALOG_SCHEMA_VERSION, create_catalog_indexes, create_catalog_tables,
    migrate_app_db, open_app_db,
};
pub use search::{BookAttrs, SearchKind, SearchQuery};
pub use summary::{ANTHOLOGY_MIN_AUTHORS, AuthorSummary, Coauthor, SeriesCount};
pub use works::{Edition, Group, edition_cmp};
