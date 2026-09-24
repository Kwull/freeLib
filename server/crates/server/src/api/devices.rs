use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::auth::Auth;
use crate::db::{self, Device, User};
use crate::error::{ApiError, ApiResult};
use crate::jobs::Job;
use crate::output::ALL_FORMATS;
use crate::sender::{self, SendRequest};
use crate::state::AppState;
use crate::util::safe_subdir;

pub async fn list(State(st): State<AppState>, Auth(u): Auth) -> ApiResult<Json<Vec<Device>>> {
    Ok(Json(st.db.run(move |c| db::list_devices(c, u.id)).await?))
}

fn validate(d: &mut Device, u: &User) -> ApiResult<()> {
    d.name = d.name.trim().to_string();
    if d.name.is_empty() || d.name.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "device name must have 1..100 characters",
        ));
    }
    if !["email", "download", "folder"].contains(&d.kind.as_str()) {
        return Err(ApiError::bad_request(
            "kind must be email, download or folder",
        ));
    }
    if !ALL_FORMATS.contains(&d.format.as_str()) {
        return Err(ApiError::bad_request("unknown format"));
    }
    if d.file_name.trim().is_empty() {
        d.file_name = db::default_file_name();
    }
    if d.file_name.len() > 500 {
        return Err(ApiError::bad_request("file name template too long"));
    }
    d.target = d
        .target
        .take()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    match d.kind.as_str() {
        "email" => {
            if let Some(t) = &d.target
                && !t.contains('@')
            {
                return Err(ApiError::bad_request("target must be an e-mail address"));
            }
        }
        "folder" => {
            if safe_subdir(d.target.as_deref().unwrap_or("")).is_none() {
                return Err(ApiError::bad_request(
                    "target must be a sub-folder of the export folder",
                ));
            }
        }
        _ => d.target = None,
    }
    if let Some(css) = &d.options.user_css
        && css.len() > 64 * 1024
    {
        return Err(ApiError::bad_request("userCss too long"));
    }
    if d.shared && !u.is_admin() {
        return Err(ApiError::forbidden(
            "only administrators can create shared devices",
        ));
    }
    d.user_id = if d.shared { None } else { Some(u.id) };
    Ok(())
}

pub async fn create(
    State(st): State<AppState>,
    Auth(u): Auth,
    Json(mut d): Json<Device>,
) -> ApiResult<Json<Device>> {
    d.id = 0;
    validate(&mut d, &u)?;
    let uid = u.id;
    let dev = st
        .db
        .run(move |c| {
            let id = db::save_device(c, &d)?;
            db::get_device(c, uid, id)
        })
        .await?;
    Ok(Json(dev))
}

pub async fn update(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<i64>,
    Json(mut d): Json<Device>,
) -> ApiResult<Json<Device>> {
    let uid = u.id;
    let existing = st.db.run(move |c| db::get_device(c, uid, id)).await?;
    if existing.shared && !u.is_admin() {
        return Err(ApiError::forbidden(
            "shared devices can only be changed by administrators",
        ));
    }
    d.id = id;
    validate(&mut d, &u)?;
    let dev = st
        .db
        .run(move |c| {
            db::save_device(c, &d)?;
            db::get_device(c, uid, id)
        })
        .await?;
    Ok(Json(dev))
}

pub async fn delete(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let uid = u.id;
    let existing = st.db.run(move |c| db::get_device(c, uid, id)).await?;
    if existing.shared && !u.is_admin() {
        return Err(ApiError::forbidden(
            "shared devices can only be deleted by administrators",
        ));
    }
    st.db.run(move |c| db::delete_device(c, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendBody {
    library: i64,
    books: Vec<i64>,
    device: i64,
    target: Option<String>,
    file_name: Option<String>,
    /// Partial `ConvertOptions`, merged (shallow, field by field) over the device's own
    /// options for this send only; the device itself is left unchanged. Lets the send dialog's
    /// "generate cover" / "join series" toggles override the device default without the user
    /// having to edit their device profile.
    options: Option<serde_json::Value>,
}

pub async fn send(
    State(st): State<AppState>,
    Auth(u): Auth,
    Json(b): Json<SendBody>,
) -> ApiResult<Json<Job>> {
    let uid = u.id;
    let dev_id = b.device;
    let mut device = st.db.run(move |c| db::get_device(c, uid, dev_id)).await?;
    if let Some(patch) = b.options {
        let patch = patch
            .as_object()
            .ok_or_else(|| ApiError::bad_request("options must be an object"))?;
        let mut val =
            serde_json::to_value(&device.options).map_err(|e| ApiError::internal(e.to_string()))?;
        if let Some(obj) = val.as_object_mut() {
            for (k, v) in patch {
                obj.insert(k.clone(), v.clone());
            }
        }
        device.options = serde_json::from_value(val)
            .map_err(|e| ApiError::bad_request(format!("invalid options: {e}")))?;
    }
    let req = SendRequest {
        library: b.library,
        books: b.books,
        device,
        target: b.target,
        file_name: b.file_name,
    };
    Ok(Json(sender::start(&st, &u, req).await?))
}

pub async fn fonts(State(st): State<AppState>, _: Auth) -> Json<Vec<String>> {
    Json(st.conv.font_names())
}
