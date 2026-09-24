use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::auth::Auth;
use crate::db::{self, Shelf};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn valid_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit())
}

fn valid_name(n: &str) -> ApiResult<String> {
    let n = n.trim();
    if n.is_empty() || n.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "shelf name must have 1..100 characters",
        ));
    }
    Ok(n.to_string())
}

pub async fn list(State(st): State<AppState>, Auth(u): Auth) -> ApiResult<Json<Vec<Shelf>>> {
    Ok(Json(st.db.run(move |c| db::list_shelves(c, u.id)).await?))
}

#[derive(Deserialize)]
pub struct ShelfBody {
    name: Option<String>,
    color: Option<String>,
}

pub async fn create(
    State(st): State<AppState>,
    Auth(u): Auth,
    Json(b): Json<ShelfBody>,
) -> ApiResult<Json<Shelf>> {
    let name = valid_name(b.name.as_deref().unwrap_or(""))?;
    let color = b.color.unwrap_or_else(|| "#1F5F5B".into());
    if !valid_color(&color) {
        return Err(ApiError::bad_request("color must be #rrggbb"));
    }
    Ok(Json(
        st.db
            .run(move |c| db::create_shelf(c, u.id, &name, &color))
            .await?,
    ))
}

pub async fn update(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<i64>,
    Json(b): Json<ShelfBody>,
) -> ApiResult<Json<Shelf>> {
    let name = b.name.as_deref().map(valid_name).transpose()?;
    if let Some(c) = &b.color
        && !valid_color(c)
    {
        return Err(ApiError::bad_request("color must be #rrggbb"));
    }
    Ok(Json(
        st.db
            .run(move |c| db::update_shelf(c, u.id, id, name.as_deref(), b.color.as_deref()))
            .await?,
    ))
}

pub async fn delete(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    st.db.run(move |c| db::delete_shelf(c, u.id, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ShelfBooks {
    library: i64,
    books: Vec<i64>,
    add: bool,
}

pub async fn books(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<i64>,
    Json(b): Json<ShelfBooks>,
) -> ApiResult<Json<Shelf>> {
    if b.books.len() > 100_000 {
        return Err(ApiError::bad_request("too many books"));
    }
    let uid = u.id;
    st.db.run(move |c| db::get_shelf(c, uid, id)).await?;
    let ids = b.books.clone();
    let keys: Vec<String> = st
        .catalog_call(b.library, move |cat| Ok(cat.keys_by_ids(&ids)?))
        .await?
        .into_iter()
        .map(|(_, k)| k)
        .collect();
    let lib = b.library;
    let add = b.add;
    Ok(Json(
        st.db
            .run(move |c| {
                db::shelf_modify(c, id, lib, &keys, add)?;
                db::get_shelf(c, uid, id)
            })
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct RatingBody {
    rating: i64,
}

pub async fn rating(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path((lib, id)): Path<(i64, i64)>,
    Json(b): Json<RatingBody>,
) -> ApiResult<StatusCode> {
    if !(0..=5).contains(&b.rating) {
        return Err(ApiError::bad_request("rating must be 0..5"));
    }
    let key = st
        .catalog_call(lib, move |cat| Ok(cat.keys_by_ids(&[id])?))
        .await?
        .pop()
        .map(|(_, k)| k)
        .ok_or_else(|| ApiError::not_found("book not found"))?;
    st.db
        .run(move |c| db::set_rating(c, u.id, lib, &key, b.rating))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
