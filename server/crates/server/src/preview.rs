//! Annotation and cover previews cached under `cache/info/<lib>/<hash>.json` and
//! `cache/covers/<lib>/<hash>-{full.<ext>,thumb.webp}`.

use std::path::{Path, PathBuf};

use freelib_catalog::BookDetail;
use serde::{Deserialize, Serialize};

use crate::bookio;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::short_hash;

pub const THUMB_HEIGHT: u32 = 240;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoCache {
    pub annotation: Option<String>,
    /// MIME type of the stored full cover, `None` = no cover.
    pub cover: Option<String>,
}

fn hash(d: &BookDetail) -> String {
    short_hash(format!("{}\0{}\0{}", d.book.key, d.book.size, d.book.ext).as_bytes())
}

fn has_metadata(ext: &str) -> bool {
    matches!(ext, "fb2" | "epub")
}

fn cover_ext(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        _ => "webp",
    }
}

fn paths(st: &AppState, lib: i64, d: &BookDetail) -> (PathBuf, PathBuf) {
    let h = hash(d);
    (
        st.cache_dir("info", lib).join(format!("{h}.json")),
        st.cache_dir("covers", lib).join(h),
    )
}

/// Annotation + cover presence (reads the book on first call, then cached).
pub async fn info(st: &AppState, lib: i64, lib_dir: &Path, d: &BookDetail) -> ApiResult<InfoCache> {
    if !has_metadata(&d.book.ext) {
        return Ok(InfoCache::default());
    }
    let (info_path, cover_base) = paths(st, lib, d);
    if let Ok(s) = tokio::fs::read(&info_path).await
        && let Ok(v) = serde_json::from_slice::<InfoCache>(&s)
    {
        return Ok(v);
    }
    let lib_dir = lib_dir.to_path_buf();
    let d = d.clone();
    let conv = st.conv.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<InfoCache> {
        let bytes = match bookio::read_original(&lib_dir, &d) {
            Ok(b) => b,
            // missing file: don't cache, it may appear later
            Err(e) if e.code == "not_found" => return Ok(InfoCache::default()),
            Err(e) => return Err(e),
        };
        let mut out = InfoCache::default();
        if let Ok(info) = conv.read_info(&bytes) {
            out.annotation = info.annotation.clone();
            if let Some(c) = info.cover {
                let (data, mime) = normalize_cover(c.data, &c.mime);
                if let Some(data) = data {
                    let p = cover_base.with_file_name(format!(
                        "{}-full.{}",
                        cover_base.file_name().unwrap_or_default().to_string_lossy(),
                        cover_ext(&mime)
                    ));
                    write_file(&p, &data)?;
                    out.cover = Some(mime);
                }
            }
        }
        let json = serde_json::to_vec(&out).map_err(|e| ApiError::internal(e.to_string()))?;
        write_file(&info_path, &json)?;
        Ok(out)
    })
    .await?
}

/// JPEG/PNG kept; other formats re-encoded to WebP.
fn normalize_cover(data: Vec<u8>, mime: &str) -> (Option<Vec<u8>>, String) {
    match mime {
        "image/jpeg" | "image/png" => (Some(data), mime.to_string()),
        _ => match image::load_from_memory(&data) {
            Ok(img) => (encode_webp(&img, 85.0), "image/webp".into()),
            Err(_) => (None, String::new()),
        },
    }
}

fn encode_webp(img: &image::DynamicImage, quality: f32) -> Option<Vec<u8>> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 || w > 16383 || h > 16383 {
        return None;
    }
    Some(
        webp::Encoder::from_rgb(rgb.as_raw(), w, h)
            .encode(quality)
            .to_vec(),
    )
}

fn write_file(p: &Path, data: &[u8]) -> ApiResult<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension(format!("tmp{}", crate::util::random_id()));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

/// Cover file and its MIME type; `None` when the book has no cover.
pub async fn cover(
    st: &AppState,
    lib: i64,
    lib_dir: &Path,
    d: &BookDetail,
    thumb: bool,
) -> ApiResult<Option<(PathBuf, String)>> {
    let mut inf = info(st, lib, lib_dir, d).await?;
    let Some(mut mime) = inf.cover.clone() else {
        return Ok(None);
    };
    let (info_path, base) = paths(st, lib, d);
    let name = base
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut full = base.with_file_name(format!("{name}-full.{}", cover_ext(&mime)));
    if !full.is_file() {
        // cache partially deleted: rebuild
        let _ = tokio::fs::remove_file(&info_path).await;
        inf = info(st, lib, lib_dir, d).await?;
        let Some(m) = inf.cover.clone() else {
            return Ok(None);
        };
        mime = m;
        full = base.with_file_name(format!("{name}-full.{}", cover_ext(&mime)));
    }
    if !thumb {
        return Ok(Some((full, mime)));
    }
    let tpath = base.with_file_name(format!("{name}-thumb.webp"));
    if tpath.is_file() {
        return Ok(Some((tpath, "image/webp".into())));
    }
    let _permit = st
        .workers
        .acquire()
        .await
        .map_err(|_| ApiError::internal("worker pool closed"))?;
    let t2 = tpath.clone();
    let made = tokio::task::spawn_blocking(move || -> ApiResult<bool> {
        let data = std::fs::read(&full)?;
        let Ok(img) = image::load_from_memory(&data) else {
            return Ok(false);
        };
        let img = if img.height() > THUMB_HEIGHT {
            let w = ((img.width() as u64 * THUMB_HEIGHT as u64) / img.height().max(1) as u64).max(1)
                as u32;
            img.resize_exact(w, THUMB_HEIGHT, image::imageops::FilterType::Triangle)
        } else {
            img
        };
        match encode_webp(&img, 80.0) {
            Some(bytes) => {
                write_file(&t2, &bytes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    })
    .await??;
    Ok(made.then_some((tpath, "image/webp".into())))
}
