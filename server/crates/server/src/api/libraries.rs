use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::auth::{Admin, Auth};
use crate::db::{self, LibraryRow};
use crate::error::{ApiError, ApiResult};
use crate::importer;
use crate::jobs::Job;
use crate::state::{AppState, LibraryDto};
use crate::util::{relative_to, resolve_inside};

pub async fn list(State(st): State<AppState>, Auth(u): Auth) -> ApiResult<Json<Vec<LibraryDto>>> {
    let st2 = st.clone();
    let v = tokio::task::spawn_blocking(move || -> ApiResult<Vec<LibraryDto>> {
        let rows = {
            let c = st2.db.lock();
            db::list_libraries(&c)?
        };
        Ok(rows.iter().map(|r| st2.library_dto(r, u.id)).collect())
    })
    .await??;
    Ok(Json(v))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LibraryBody {
    name: Option<String>,
    path: Option<String>,
    inpx: Option<Option<String>>,
    first_author_only: Option<bool>,
    skip_deleted: Option<bool>,
    is_default: Option<bool>,
}

/// Library folder and INPX must be inside `FREELIB_BOOKS_DIR`; stored as absolute paths.
fn validate_paths(books: &Path, path: &str, inpx: Option<&str>) -> ApiResult<(String, Option<String>)> {
    let dir = resolve_inside(books, path)?;
    if !dir.is_dir() {
        return Err(ApiError::bad_request("library path must be a folder"));
    }
    let inpx = match inpx.map(str::trim).filter(|s| !s.is_empty()) {
        Some(i) => {
            // relative INPX paths are relative to the books folder (as /fs returns them);
            // a bare file name is looked up in the library folder
            let candidate = if !Path::new(i).is_absolute() && !i.contains('/') && dir.join(i).is_file() {
                dir.join(i).to_string_lossy().into_owned()
            } else {
                i.to_string()
            };
            let f = resolve_inside(books, &candidate)?;
            if !f.is_file() {
                return Err(ApiError::bad_request("INPX must be a file"));
            }
            Some(f.to_string_lossy().into_owned())
        }
        None => None,
    };
    Ok((dir.to_string_lossy().into_owned(), inpx))
}

pub async fn create(State(st): State<AppState>, Admin(u): Admin, Json(b): Json<LibraryBody>) -> ApiResult<Json<LibraryDto>> {
    let name = b.name.as_deref().map(str::trim).unwrap_or("").to_string();
    let path = b.path.clone().unwrap_or_default();
    let inpx = b.inpx.clone().flatten();
    let (path, inpx) = validate_paths(&st.cfg.books_dir, &path, inpx.as_deref())?;
    let name = if name.is_empty() {
        Path::new(inpx.as_deref().unwrap_or(&path))
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Library".into())
    } else {
        name
    };
    let row = LibraryRow {
        id: 0,
        name,
        path,
        inpx: inpx.clone(),
        first_author_only: b.first_author_only.unwrap_or(false),
        skip_deleted: b.skip_deleted.unwrap_or(false),
        is_default: b.is_default.unwrap_or(false),
    };
    let id = st.db.run(move |c| db::insert_library(c, &row)).await?;
    st.add_lib(id);
    if inpx.is_some() {
        importer::start(&st, id, &u).await?;
    }
    Ok(Json(st.library_dto_async(id, u.id).await?))
}

pub async fn update(
    State(st): State<AppState>,
    Admin(u): Admin,
    UrlPath(id): UrlPath<i64>,
    Json(b): Json<LibraryBody>,
) -> ApiResult<Json<LibraryDto>> {
    let mut row = st.db.run(move |c| db::get_library(c, id)).await?.ok_or_else(|| ApiError::not_found("library not found"))?;
    if let Some(n) = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        row.name = n.to_string();
    }
    let path_changed = b.path.as_ref().is_some_and(|p| *p != row.path);
    let inpx_changed = b.inpx.as_ref().is_some_and(|i| *i != row.inpx);
    if path_changed || inpx_changed {
        let path = b.path.clone().unwrap_or(row.path.clone());
        let inpx = b.inpx.clone().unwrap_or(row.inpx.clone());
        let (p, i) = validate_paths(&st.cfg.books_dir, &path, inpx.as_deref())?;
        row.path = p;
        row.inpx = i;
    }
    if let Some(v) = b.first_author_only {
        row.first_author_only = v;
    }
    if let Some(v) = b.skip_deleted {
        row.skip_deleted = v;
    }
    if let Some(v) = b.is_default {
        row.is_default = v;
    }
    st.db.run(move |c| db::update_library(c, &row)).await?;
    st.emit_library(id);
    Ok(Json(st.library_dto_async(id, u.id).await?))
}

pub async fn delete(State(st): State<AppState>, Admin(_): Admin, UrlPath(id): UrlPath<i64>) -> ApiResult<StatusCode> {
    let rt = st.lib(id)?;
    if rt.import.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        return Err(ApiError::conflict("an import of this library is running"));
    }
    st.db.run(move |c| db::delete_library(c, id)).await?;
    rt.handle.close();
    st.remove_lib(id);
    let db_path = rt.handle.path().to_path_buf();
    let caches: Vec<PathBuf> = ["info", "covers", "out"].iter().map(|k| st.cache_dir(k, id)).collect();
    tokio::task::spawn_blocking(move || {
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(freelib_import::new_db_path(&db_path));
        for c in caches {
            let _ = std::fs::remove_dir_all(c);
        }
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
pub struct ImportBody {
    #[serde(default)]
    #[allow(dead_code)]
    mode: Option<String>,
}

/// Mode "new" is a full rebuild too (see ARCHITECTURE.md "Import").
pub async fn import(
    State(st): State<AppState>,
    Admin(u): Admin,
    UrlPath(id): UrlPath<i64>,
    body: Option<Json<ImportBody>>,
) -> ApiResult<Json<Job>> {
    if let Some(Json(b)) = &body
        && let Some(m) = &b.mode
        && m != "full"
        && m != "new"
    {
        return Err(ApiError::bad_request("mode must be 'full' or 'new'"));
    }
    Ok(Json(importer::start(&st, id, &u).await?))
}

#[derive(Deserialize)]
pub struct FsQuery {
    path: Option<String>,
}

pub async fn fs(State(st): State<AppState>, Admin(_): Admin, Query(q): Query<FsQuery>) -> ApiResult<Json<serde_json::Value>> {
    let books = st.cfg.books_dir.clone();
    let p = q.path.unwrap_or_default();
    let v = tokio::task::spawn_blocking(move || -> ApiResult<serde_json::Value> {
        let root = books.canonicalize().map_err(|_| ApiError::not_found("books folder is not available"))?;
        let dir = resolve_inside(&root, if p.trim().is_empty() { "." } else { &p })?;
        if !dir.is_dir() {
            return Err(ApiError::bad_request("not a folder"));
        }
        let mut entries = Vec::new();
        for e in std::fs::read_dir(&dir)?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            // follow symlinks but hide those pointing outside the books folder
            let Ok(real) = e.path().canonicalize() else { continue };
            if !real.starts_with(&root) {
                continue;
            }
            let Ok(md) = std::fs::metadata(&real) else { continue };
            let lower = name.to_lowercase();
            if md.is_dir() {
                entries.push((true, name, 0u64));
            } else if lower.ends_with(".inpx") || lower.ends_with(".zip") {
                entries.push((false, name, md.len()));
            }
        }
        entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
        let rel = relative_to(&root, &dir);
        let parent = if dir == root { None } else { dir.parent().map(|p| relative_to(&root, p)) };
        Ok(json!({
            "path": rel,
            "parent": parent,
            "entries": entries.into_iter().map(|(d, n, s)| json!({"name": n, "dir": d, "size": s})).collect::<Vec<_>>(),
        }))
    })
    .await??;
    Ok(Json(v))
}
