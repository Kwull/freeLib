//! "Send to my phone": short-lived download links for one book.
//!
//! `POST /api/v1/handoff` (signed-in user) creates a link `/h/<token>` for one book in one
//! format with one device's conversion options. Opening it on a phone shows a minimal page
//! (cover, title, an "Open in Books" / "Download" button) without signing in; the button
//! downloads the file with `Content-Type: application/epub+zip` and
//! `Content-Disposition: attachment`, which Safari on iPhone/iPad offers to open in Books.
//!
//! Security properties:
//! * the token is 128 random bits (base64url, 22 characters); only its SHA-256 is stored, so
//!   a copy of `app.db` does not reveal working links;
//! * a link expires after [`TTL_SECS`] (15 minutes) and allows [`MAX_USES`] downloads (viewing
//!   the page or the cover does not count; `HEAD` does not count);
//! * it is bound to one book, one format and the options chosen at creation; it grants nothing
//!   else: no session, no other book, no API access;
//! * it belongs to the user who created it: deleting the user deletes their links, and
//!   downloads are recorded in that user's history;
//! * creation is rate limited to [`MAX_PER_WINDOW`] links per user per 10 minutes (429);
//! * the page and file are `Cache-Control: no-store`, `Referrer-Policy: no-referrer`,
//!   `X-Robots-Tag: noindex`, and the page has a strict CSP (no scripts at all).
//!
//! Anyone who gets the URL (or photographs the QR code) within those 15 minutes can download
//! that one book — the same trust as handing someone the file.

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::books::{cover_response, file_response, lib_dir, load_book};
use crate::auth::Auth;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::output;
use crate::state::AppState;
use crate::util::{random_token, rfc3339_at, set_header, sha256_hex, unix_now};

/// Lifetime of a link.
pub const TTL_SECS: i64 = 15 * 60;
/// Downloads per link (a phone may need a second try).
pub const MAX_USES: i64 = 3;
/// Links a user may create per 10 minutes.
pub const MAX_PER_WINDOW: i64 = 20;

/// The public routes (`/h/...`, no login).
pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/h/{token}", get(page))
        .route("/h/{token}/cover", get(cover))
        .route("/h/{token}/file", get(file))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBody {
    library: i64,
    book: i64,
    /// Device whose format and options to use; default: the Apple Books device, else EPUB.
    device: Option<i64>,
    /// Overrides the device's format.
    format: Option<String>,
}

/// Base URL for links shown to the user: `FREELIB_PUBLIC_URL`, else the request's host.
fn base_url(st: &AppState, headers: &HeaderMap) -> String {
    if let Some(u) = &st.cfg.public_url {
        return u.trim_end_matches('/').to_string();
    }
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .unwrap_or("localhost:8080");
    let scheme = if crate::auth::is_https(headers) {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://{host}")
}

/// `POST /api/v1/handoff`: `{library, book, device?, format?}` →
/// `{url, absoluteUrl, expiresAt, maxUses, format, fileName, title}`.
pub async fn create(
    State(st): State<AppState>,
    Auth(u): Auth,
    headers: HeaderMap,
    Json(b): Json<CreateBody>,
) -> ApiResult<Json<Value>> {
    let (_, d) = load_book(&st, b.library, b.book).await?;
    let uid = u.id;
    let devs = st.db.run(move |c| db::list_devices(c, uid)).await?;
    let dev = match b.device {
        Some(id) => Some(
            devs.iter()
                .find(|d| d.id == id)
                .cloned()
                .ok_or_else(|| ApiError::not_found("device not found"))?,
        ),
        None => devs
            .iter()
            .find(|d| d.preset.as_deref() == Some("apple-books"))
            .cloned(),
    };
    let dev_format = dev
        .as_ref()
        .filter(|d| d.kind != "email")
        .map(|d| d.format.clone());
    let mut format = b
        .format
        .clone()
        .or(dev_format)
        .unwrap_or_else(|| "epub".into());
    // a phone cannot use what only Calibre makes when Calibre is missing: fall back to EPUB
    if output::check_format(&st, &d.book.ext, &format).is_err() && b.format.is_none() {
        format = "epub".into();
    }
    output::check_format(&st, &d.book.ext, &format)?;
    let opts = dev.as_ref().map(|d| d.options.clone()).unwrap_or_default();
    let template = dev
        .as_ref()
        .map(|d| d.file_name.clone())
        .unwrap_or_else(db::default_file_name);
    let file_name =
        output::download_name(&st, &template, &d.book, &format, opts.transliterate, true);
    let token = random_token(16);
    let hash = sha256_hex(token.as_bytes());
    let now = unix_now();
    let expires = now + TTL_SECS;
    let (lib, book, key) = (b.library, d.book.id, d.book.key.clone());
    let (fmt, name) = (format.clone(), file_name.clone());
    let dev_id = dev.as_ref().map(|d| d.id);
    let opts_json = serde_json::to_string(&opts).map_err(|e| ApiError::internal(e.to_string()))?;
    st.db
        .run(move |c| {
            c.execute(
                "DELETE FROM handoff WHERE expires_at < ?1",
                [now - 3600],
            )?;
            let recent: i64 = c.query_row(
                "SELECT count(*) FROM handoff WHERE user_id=?1 AND created_at > ?2",
                rusqlite::params![uid, now - 600],
                |r| r.get(0),
            )?;
            if recent >= MAX_PER_WINDOW {
                return Err(ApiError::rate_limited(
                    "too many phone links; try again in a few minutes",
                ));
            }
            c.execute(
                "INSERT INTO handoff(token_hash, user_id, library_id, book_id, book_key, device_id, format, options, file_name, created_at, expires_at, uses, max_uses) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0,?12)",
                rusqlite::params![hash, uid, lib, book, key, dev_id, fmt, opts_json, name, now, expires, MAX_USES],
            )?;
            Ok(())
        })
        .await?;
    let url = format!("/h/{token}");
    Ok(Json(json!({
        "url": url,
        "absoluteUrl": format!("{}{url}", base_url(&st, &headers)),
        "expiresAt": rfc3339_at(expires),
        "maxUses": MAX_USES,
        "format": format,
        "fileName": file_name,
        "title": d.book.title,
    })))
}

struct Link {
    user_id: i64,
    library: i64,
    book: i64,
    book_key: String,
    format: String,
    options: String,
    file_name: String,
    expires_at: i64,
    uses: i64,
    max_uses: i64,
}

enum Lookup {
    Valid(Link),
    Expired,
    UsedUp,
    Unknown,
}

async fn lookup(st: &AppState, token: &str) -> ApiResult<Lookup> {
    // tokens are 22 base64url characters; anything else cannot match
    if token.len() != 22
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Ok(Lookup::Unknown);
    }
    let hash = sha256_hex(token.as_bytes());
    let row = st
        .db
        .run(move |c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row(
                "SELECT user_id, library_id, book_id, book_key, format, options, file_name, expires_at, uses, max_uses \
                 FROM handoff WHERE token_hash=?1",
                [hash],
                |r| {
                    Ok(Link {
                        user_id: r.get(0)?,
                        library: r.get(1)?,
                        book: r.get(2)?,
                        book_key: r.get(3)?,
                        format: r.get(4)?,
                        options: r.get(5)?,
                        file_name: r.get(6)?,
                        expires_at: r.get(7)?,
                        uses: r.get(8)?,
                        max_uses: r.get(9)?,
                    })
                },
            )
            .optional()?)
        })
        .await?;
    Ok(match row {
        None => Lookup::Unknown,
        Some(l) if l.expires_at <= unix_now() => Lookup::Expired,
        Some(l) if l.uses >= l.max_uses => Lookup::UsedUp,
        Some(l) => Lookup::Valid(l),
    })
}

/// Page language from `Accept-Language`: `ru`, `uk` or `en`.
fn page_lang(headers: &HeaderMap) -> &'static str {
    let al = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let first = al.split(',').next().unwrap_or("").trim();
    if first.starts_with("ru") {
        "ru"
    } else if first.starts_with("uk") {
        "uk"
    } else {
        "en"
    }
}

fn is_ios(headers: &HeaderMap) -> bool {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ua| ua.contains("iPhone") || ua.contains("iPad") || ua.contains("iPod"))
}

struct Texts {
    open_books: &'static str,
    download: &'static str,
    ios_hint: &'static str,
    other_hint: &'static str,
    minutes_left: &'static str,
    downloads_left: &'static str,
    expired_title: &'static str,
    expired_text: &'static str,
    used_title: &'static str,
}

fn texts(lang: &str) -> Texts {
    match lang {
        "ru" => Texts {
            open_books: "Открыть в Книгах",
            download: "Скачать",
            ios_hint: "Safari загрузит книгу и предложит открыть её в «Книгах».",
            other_hint: "Откройте файл в приложении для чтения.",
            minutes_left: "Ссылка действует ещё {m} мин",
            downloads_left: "загрузок осталось: {n}",
            expired_title: "Ссылка устарела",
            expired_text: "Ссылки на телефон действуют 15 минут. Создайте новую на компьютере: «Отправить на телефон».",
            used_title: "Ссылка уже использована",
        },
        "uk" => Texts {
            open_books: "Відкрити в Книгах",
            download: "Завантажити",
            ios_hint: "Safari завантажить книгу й запропонує відкрити її в «Книгах».",
            other_hint: "Відкрийте файл у застосунку для читання.",
            minutes_left: "Посилання діє ще {m} хв",
            downloads_left: "завантажень залишилося: {n}",
            expired_title: "Посилання застаріло",
            expired_text: "Посилання на телефон діють 15 хвилин. Створіть нове на комп’ютері: «Надіслати на телефон».",
            used_title: "Посилання вже використано",
        },
        _ => Texts {
            open_books: "Open in Books",
            download: "Download",
            ios_hint: "Safari downloads the book and offers to open it in Books.",
            other_hint: "Open the file with your reading app.",
            minutes_left: "Link valid for {m} more min",
            downloads_left: "{n} downloads left",
            expired_title: "This link has expired",
            expired_text: "Phone links work for 15 minutes. Create a new one on your computer with “Send to my phone”.",
            used_title: "This link was already used",
        },
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const PAGE_CSP: &str = "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const STYLE: &str = "\
:root{color-scheme:light dark;--bg:#f6f3ee;--card:#fff;--ink:#1d1c19;--muted:#6b675f;--accent:#1f5f5b}\
@media (prefers-color-scheme:dark){:root{--bg:#161615;--card:#22211f;--ink:#efece6;--muted:#a19c93;--accent:#5fb3a9}}\
*{box-sizing:border-box}body{margin:0;min-height:100vh;background:var(--bg);color:var(--ink);\
font:16px/1.45 -apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;display:flex;align-items:center;justify-content:center;padding:24px 16px}\
main{width:100%;max-width:420px;background:var(--card);border-radius:18px;padding:28px 22px;text-align:center;box-shadow:0 10px 30px rgba(0,0,0,.08)}\
img{display:block;margin:0 auto 18px;max-width:62%;max-height:42vh;border-radius:6px;box-shadow:0 6px 20px rgba(0,0,0,.25)}\
h1{font:600 22px/1.25 Georgia,'Times New Roman',serif;margin:0 0 6px;overflow-wrap:anywhere}\
.by{color:var(--muted);margin:0 0 4px}.series{color:var(--muted);font-style:italic;margin:0}\
a.btn{display:block;margin:22px 0 10px;padding:15px 18px;border-radius:12px;background:var(--accent);color:#fff;\
font-weight:600;font-size:18px;text-decoration:none}\
.hint{color:var(--muted);font-size:14px;margin:0}.meta{color:var(--muted);font-size:13px;margin:14px 0 0}\
.brand{color:var(--muted);font-size:12px;margin-top:18px;letter-spacing:.04em}";

fn html_page(lang: &str, title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex,nofollow\"><title>{}</title><style>{STYLE}</style></head>\
         <body><main>{body}<p class=\"brand\">freeLib</p></main></body></html>",
        esc(title)
    )
}

fn private_headers(r: &mut Response) {
    set_header(r, header::CACHE_CONTROL, "no-store");
    set_header(
        r,
        header::HeaderName::from_static("referrer-policy"),
        "no-referrer",
    );
    set_header(
        r,
        header::HeaderName::from_static("x-robots-tag"),
        "noindex, nofollow",
    );
}

fn gone_page(lang: &str, used: bool) -> Response {
    let t = texts(lang);
    let title = if used { t.used_title } else { t.expired_title };
    let body = format!(
        "<h1>{}</h1><p class=\"hint\">{}</p>",
        esc(title),
        esc(t.expired_text)
    );
    let mut r = (StatusCode::GONE, html_page(lang, title, &body)).into_response();
    set_header(&mut r, header::CONTENT_TYPE, "text/html; charset=utf-8");
    set_header(&mut r, header::CONTENT_SECURITY_POLICY, PAGE_CSP);
    private_headers(&mut r);
    r
}

/// `GET /h/:token`: the landing page on the phone.
async fn page(
    State(st): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let lang = page_lang(&headers);
    let link = match lookup(&st, &token).await? {
        Lookup::Valid(l) => l,
        Lookup::UsedUp => return Ok(gone_page(lang, true)),
        Lookup::Expired | Lookup::Unknown => return Ok(gone_page(lang, false)),
    };
    let (_, d) = load_book(&st, link.library, link.book).await?;
    let t = texts(lang);
    let ios = is_ios(&headers);
    let epub = matches!(link.format.as_str(), "epub" | "kepub");
    let (label, hint) = if ios && epub {
        (t.open_books, t.ios_hint)
    } else {
        (t.download, t.other_hint)
    };
    let authors = d
        .book
        .authors
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let series = d
        .book
        .series
        .as_ref()
        .map(|s| match d.book.serno {
            Some(n) if n > 0 => format!("{} · {n}", s.name),
            _ => s.name.clone(),
        })
        .unwrap_or_default();
    let minutes = ((link.expires_at - unix_now()) as f64 / 60.0)
        .ceil()
        .max(1.0) as i64;
    let fmt = if link.format == "original" {
        d.book.ext.to_uppercase()
    } else {
        link.format.to_uppercase()
    };
    let meta = format!(
        "{} · {} · {}",
        fmt,
        t.minutes_left.replace("{m}", &minutes.to_string()),
        t.downloads_left
            .replace("{n}", &(link.max_uses - link.uses).to_string())
    );
    let mut body = format!(
        "<img src=\"/h/{tok}/cover\" alt=\"\" width=\"240\" height=\"360\"><h1>{}</h1>",
        esc(&d.book.title),
        tok = esc(&token)
    );
    if !authors.is_empty() {
        body.push_str(&format!("<p class=\"by\">{}</p>", esc(&authors)));
    }
    if !series.is_empty() {
        body.push_str(&format!("<p class=\"series\">{}</p>", esc(&series)));
    }
    body.push_str(&format!(
        "<a class=\"btn\" href=\"/h/{tok}/file\" data-testid=\"handoff-download\">{}</a><p class=\"hint\">{}</p><p class=\"meta\">{}</p>",
        esc(label),
        esc(hint),
        esc(&meta),
        tok = esc(&token)
    ));
    let mut r = Response::new(Body::from(html_page(lang, &d.book.title, &body)));
    set_header(&mut r, header::CONTENT_TYPE, "text/html; charset=utf-8");
    set_header(&mut r, header::CONTENT_SECURITY_POLICY, PAGE_CSP);
    private_headers(&mut r);
    Ok(r)
}

/// `GET /h/:token/cover`: the book's cover (or placeholder) while the link is valid.
async fn cover(State(st): State<AppState>, Path(token): Path<String>) -> ApiResult<Response> {
    let Lookup::Valid(link) = lookup(&st, &token).await? else {
        return Err(ApiError::new(StatusCode::GONE, "not_found", "link expired"));
    };
    let (_, d) = load_book(&st, link.library, link.book).await?;
    let dir = lib_dir(&st, link.library).await?;
    let mut r = cover_response(&st, link.library, &dir, &d, true, None).await?;
    private_headers(&mut r);
    Ok(r)
}

/// `GET /h/:token/file`: the book (counts one use; `HEAD` does not).
async fn file(
    State(st): State<AppState>,
    Path(token): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let link = match lookup(&st, &token).await? {
        Lookup::Valid(l) => l,
        Lookup::UsedUp => return Ok(gone_page(page_lang(&headers), true)),
        _ => return Ok(gone_page(page_lang(&headers), false)),
    };
    let hash = sha256_hex(token.as_bytes());
    let counted = method != Method::HEAD;
    if counted {
        let h = hash.clone();
        let now = unix_now();
        let n = st
            .db
            .run(move |c| {
                Ok(c.execute(
                    "UPDATE handoff SET uses=uses+1 WHERE token_hash=?1 AND uses<max_uses AND expires_at>?2",
                    rusqlite::params![h, now],
                )?)
            })
            .await?;
        if n == 0 {
            return Ok(gone_page(page_lang(&headers), true));
        }
    }
    let result = async {
        let (_, d) = load_book(&st, link.library, link.book).await?;
        let dir = lib_dir(&st, link.library).await?;
        let opts: freelib_fb2conv::ConvertOptions =
            serde_json::from_str(&link.options).unwrap_or_default();
        let produced =
            output::produce(&st, link.library, &dir, &d, &link.format, &opts, None).await?;
        let ext = output::file_ext(&link.format, &d.book.ext);
        file_response(produced, &link.file_name, &ext, false).await
    }
    .await;
    match result {
        Ok(mut r) => {
            private_headers(&mut r);
            if counted {
                let (uid, lib, key) = (link.user_id, link.library, link.book_key.clone());
                let _ = st
                    .db
                    .run(move |c| db::add_history(c, uid, lib, &[key], "download", Some("phone")))
                    .await;
            }
            Ok(r)
        }
        Err(e) => {
            // a failed conversion does not use up the link
            if counted {
                let _ = st
                    .db
                    .run(move |c| {
                        Ok(c.execute(
                            "UPDATE handoff SET uses=max(uses-1, 0) WHERE token_hash=?1",
                            [hash],
                        )?)
                    })
                    .await;
            }
            Err(e)
        }
    }
}
