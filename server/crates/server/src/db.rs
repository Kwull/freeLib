//! `app.db` access: users, sessions, libraries, devices, shelves, ratings, settings, prefs.
//!
//! One connection behind a mutex; async code goes through [`AppDb::run`] (spawn_blocking).

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

use freelib_fb2conv::ConvertOptions;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::util::{now_rfc3339, rfc3339_at, sha256_hex, unix_now};

pub const SESSION_DAYS: i64 = 30;

#[derive(Clone)]
pub struct AppDb {
    conn: Arc<Mutex<Connection>>,
}

impl AppDb {
    pub fn open(path: &Path) -> anyhow::Result<AppDb> {
        let conn = freelib_catalog::open_app_db(path)?;
        rusqlite::vtab::array::load_module(&conn)?;
        Ok(AppDb {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Blocking access (inside `spawn_blocking` or at startup).
    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub async fn run<R, F>(&self, f: F) -> ApiResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> ApiResult<R> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = db.lock();
            f(&mut c)
        })
        .await?
    }
}

fn text_array<S: AsRef<str>>(v: &[S]) -> Rc<Vec<Value>> {
    Rc::new(
        v.iter()
            .map(|s| Value::Text(s.as_ref().to_string()))
            .collect(),
    )
}

// ---------------------------------------------------------------- users & sessions

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: String,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
    /// The implicit user of open mode.
    pub fn open_mode_admin() -> User {
        User {
            id: 0,
            username: "admin".into(),
            role: "admin".into(),
        }
    }
}

pub fn count_users(c: &Connection) -> ApiResult<i64> {
    Ok(c.query_row("SELECT count(*) FROM user", [], |r| r.get(0))?)
}

pub fn count_admins(c: &Connection) -> ApiResult<i64> {
    Ok(
        c.query_row("SELECT count(*) FROM user WHERE role='admin'", [], |r| {
            r.get(0)
        })?,
    )
}

pub fn list_users(c: &Connection) -> ApiResult<Vec<User>> {
    let mut st = c.prepare("SELECT id, username, role FROM user ORDER BY username")?;
    let rows = st.query_map([], |r| {
        Ok(User {
            id: r.get(0)?,
            username: r.get(1)?,
            role: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get_user(c: &Connection, id: i64) -> ApiResult<Option<User>> {
    Ok(c.query_row(
        "SELECT id, username, role FROM user WHERE id=?1",
        [id],
        |r| {
            Ok(User {
                id: r.get(0)?,
                username: r.get(1)?,
                role: r.get(2)?,
            })
        },
    )
    .optional()?)
}

/// User and password hash by (case-insensitive) user name.
pub fn user_with_hash(c: &Connection, username: &str) -> ApiResult<Option<(User, String)>> {
    Ok(c.query_row(
        "SELECT id, username, role, password_hash FROM user WHERE username=?1 COLLATE NOCASE",
        [username],
        |r| {
            Ok((
                User {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    role: r.get(2)?,
                },
                r.get(3)?,
            ))
        },
    )
    .optional()?)
}

/// Inserts a user. Names are unique case-insensitively (409 otherwise); ids are never reused
/// (a high-water mark in `setting.last_user_id`), so a deleted user's in-memory jobs, cached
/// sessions or files can never be mistaken for a new user's.
pub fn insert_user(c: &Connection, username: &str, hash: &str, role: &str) -> ApiResult<User> {
    let first = count_users(c)? == 0;
    if user_with_hash(c, username)?.is_some() {
        return Err(ApiError::conflict("user name already exists"));
    }
    let max_id: i64 = c.query_row("SELECT coalesce(max(id), 0) FROM user", [], |r| r.get(0))?;
    let hwm = get_setting_raw(c, "last_user_id")?
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let id = max_id.max(hwm) + 1;
    c.execute(
        "INSERT INTO user(id, username, password_hash, role, created_at) VALUES (?1,?2,?3,?4,?5)",
        params![id, username, hash, role, now_rfc3339()],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            ApiError::conflict("user name already exists")
        }
        e => e.into(),
    })?;
    put_setting_raw(c, "last_user_id", &id.to_string())?;
    if first {
        adopt_open_mode_data(c, id)?;
    }
    Ok(User {
        id,
        username: username.into(),
        role: role.into(),
    })
}

/// Data created in open mode belongs to user 0; the first real user takes it over.
fn adopt_open_mode_data(c: &Connection, id: i64) -> ApiResult<()> {
    c.execute("UPDATE shelf SET user_id=?1 WHERE user_id=0", [id])?;
    c.execute(
        "UPDATE OR IGNORE rating SET user_id=?1 WHERE user_id=0",
        [id],
    )?;
    c.execute("UPDATE device SET user_id=?1 WHERE user_id=0", [id])?;
    c.execute(
        "UPDATE OR IGNORE user_state SET user_id=?1 WHERE user_id=0",
        [id],
    )?;
    Ok(())
}

pub fn update_user(
    c: &Connection,
    id: i64,
    hash: Option<&str>,
    role: Option<&str>,
) -> ApiResult<()> {
    if let Some(h) = hash {
        c.execute(
            "UPDATE user SET password_hash=?1 WHERE id=?2",
            params![h, id],
        )?;
        // a new password logs out all sessions of that user
        c.execute("DELETE FROM session WHERE user_id=?1", [id])?;
    }
    if let Some(r) = role {
        c.execute("UPDATE user SET role=?1 WHERE id=?2", params![r, id])?;
    }
    Ok(())
}

pub fn delete_user(c: &Connection, id: i64) -> ApiResult<bool> {
    c.execute("DELETE FROM session WHERE user_id=?1", [id])?;
    c.execute("DELETE FROM shelf WHERE user_id=?1", [id])?;
    c.execute("DELETE FROM rating WHERE user_id=?1", [id])?;
    c.execute("DELETE FROM device WHERE user_id=?1", [id])?;
    c.execute("DELETE FROM user_state WHERE user_id=?1", [id])?;
    c.execute("DELETE FROM mail_count WHERE user_id=?1", [id])?;
    c.execute("DELETE FROM user_identity WHERE user_id=?1", [id])?;
    Ok(c.execute("DELETE FROM user WHERE id=?1", [id])? > 0)
}

/// Creates a session; returns the raw token (only its SHA-256 is stored).
pub fn create_session(c: &Connection, user_id: i64, token: &str) -> ApiResult<()> {
    let expires = rfc3339_at(unix_now() + SESSION_DAYS * 86_400);
    c.execute("DELETE FROM session WHERE expires_at < ?1", [now_rfc3339()])?;
    c.execute(
        "INSERT INTO session(token, user_id, expires_at) VALUES (?1,?2,?3)",
        params![sha256_hex(token.as_bytes()), user_id, expires],
    )?;
    Ok(())
}

pub fn session_user(c: &Connection, token: &str) -> ApiResult<Option<User>> {
    let hash = sha256_hex(token.as_bytes());
    let row: Option<(String, i64, String, String, String)> = c
        .query_row(
            "SELECT s.token, u.id, u.username, u.role, s.expires_at FROM session s JOIN user u ON u.id = s.user_id WHERE s.token=?1",
            [&hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((stored, id, username, role, expires)) = row else {
        return Ok(None);
    };
    // the lookup is by hash already; compare in constant time anyway
    if !bool::from(subtle::ConstantTimeEq::ct_eq(
        stored.as_bytes(),
        hash.as_bytes(),
    )) {
        return Ok(None);
    }
    if expires < now_rfc3339() {
        c.execute("DELETE FROM session WHERE token=?1", [&hash])?;
        return Ok(None);
    }
    Ok(Some(User { id, username, role }))
}

pub fn delete_session(c: &Connection, token: &str) -> ApiResult<()> {
    c.execute(
        "DELETE FROM session WHERE token=?1",
        [sha256_hex(token.as_bytes())],
    )?;
    Ok(())
}

/// Deletes all sessions of a user except `keep` (a raw token).
pub fn delete_other_sessions(c: &Connection, user_id: i64, keep: &str) -> ApiResult<()> {
    c.execute(
        "DELETE FROM session WHERE user_id=?1 AND token<>?2",
        params![user_id, sha256_hex(keep.as_bytes())],
    )?;
    Ok(())
}

/// Whether the user can sign in with a password (single sign-on accounts start without one).
pub fn has_password(c: &Connection, user_id: i64) -> ApiResult<bool> {
    Ok(c.query_row(
        "SELECT password_hash <> '' FROM user WHERE id=?1",
        [user_id],
        |r| r.get::<_, bool>(0),
    )
    .optional()?
    .unwrap_or(false))
}

pub fn password_hash(c: &Connection, user_id: i64) -> ApiResult<Option<String>> {
    Ok(c.query_row(
        "SELECT password_hash FROM user WHERE id=?1",
        [user_id],
        |r| r.get(0),
    )
    .optional()?)
}

// ---------------------------------------------------------------- single sign-on identities

/// A linked OpenID Connect identity.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    #[serde(skip)]
    pub user_id: i64,
    pub issuer: String,
    #[serde(skip)]
    pub subject: String,
    pub email: Option<String>,
    pub created_at: String,
    pub last_login: Option<String>,
}

fn identity_row(r: &rusqlite::Row) -> rusqlite::Result<Identity> {
    Ok(Identity {
        user_id: r.get(0)?,
        issuer: r.get(1)?,
        subject: r.get(2)?,
        email: r.get(3)?,
        created_at: r.get(4)?,
        last_login: r.get(5)?,
    })
}

const IDENTITY_COLS: &str = "user_id, issuer, subject, email, created_at, last_login";

/// The user linked to (`issuer`, `subject`).
pub fn identity_user(c: &Connection, issuer: &str, subject: &str) -> ApiResult<Option<User>> {
    Ok(c.query_row(
        "SELECT u.id, u.username, u.role FROM user_identity i JOIN user u ON u.id = i.user_id \
         WHERE i.issuer=?1 AND i.subject=?2",
        params![issuer, subject],
        |r| {
            Ok(User {
                id: r.get(0)?,
                username: r.get(1)?,
                role: r.get(2)?,
            })
        },
    )
    .optional()?)
}

/// The identity of `user_id` at `issuer`.
pub fn user_identity(c: &Connection, user_id: i64, issuer: &str) -> ApiResult<Option<Identity>> {
    Ok(c.query_row(
        &format!("SELECT {IDENTITY_COLS} FROM user_identity WHERE user_id=?1 AND issuer=?2"),
        params![user_id, issuer],
        identity_row,
    )
    .optional()?)
}

/// All identities by user id (the admin's user list).
pub fn identities(c: &Connection) -> ApiResult<HashMap<i64, Identity>> {
    let mut st = c.prepare(&format!("SELECT {IDENTITY_COLS} FROM user_identity"))?;
    let rows = st.query_map([], identity_row)?;
    let mut out = HashMap::new();
    for r in rows {
        let r = r?;
        out.insert(r.user_id, r);
    }
    Ok(out)
}

/// Links (`issuer`, `subject`) to `user_id`, replacing the user's previous identity at that
/// issuer. 409 when the identity belongs to another user.
pub fn link_identity(
    c: &Connection,
    user_id: i64,
    issuer: &str,
    subject: &str,
    email: Option<&str>,
) -> ApiResult<()> {
    if let Some(u) = identity_user(c, issuer, subject)? {
        if u.id != user_id {
            return Err(ApiError::conflict(
                "this sign-in is already linked to another account",
            ));
        }
        return Ok(());
    }
    let tx = c.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM user_identity WHERE user_id=?1 AND issuer=?2",
        params![user_id, issuer],
    )?;
    tx.execute(
        "INSERT INTO user_identity(issuer, subject, user_id, email, created_at) VALUES (?1,?2,?3,?4,?5)",
        params![issuer, subject, user_id, email, now_rfc3339()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Records a sign-in through (`issuer`, `subject`).
pub fn touch_identity(
    c: &Connection,
    issuer: &str,
    subject: &str,
    email: Option<&str>,
) -> ApiResult<()> {
    c.execute(
        "UPDATE user_identity SET last_login=?3, email=coalesce(?4, email) WHERE issuer=?1 AND subject=?2",
        params![issuer, subject, now_rfc3339(), email],
    )?;
    Ok(())
}

/// Removes the identities of `user_id`; whether there was one.
pub fn unlink_identity(c: &Connection, user_id: i64) -> ApiResult<bool> {
    Ok(c.execute("DELETE FROM user_identity WHERE user_id=?1", [user_id])? > 0)
}

// ---------------------------------------------------------------- user state

pub fn last_visit(c: &Connection, user_id: i64) -> ApiResult<Option<String>> {
    Ok(c.query_row(
        "SELECT last_visit FROM user_state WHERE user_id=?1",
        [user_id],
        |r| r.get(0),
    )
    .optional()?
    .flatten())
}

pub fn set_last_visit(c: &Connection, user_id: i64, when: &str) -> ApiResult<()> {
    c.execute(
        "INSERT INTO user_state(user_id, last_visit) VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET last_visit=excluded.last_visit",
        params![user_id, when],
    )?;
    Ok(())
}

/// Start of the previous visit (baseline of `newSinceLastVisit`).
pub fn prev_visit(c: &Connection, user_id: i64) -> ApiResult<Option<String>> {
    Ok(c.query_row(
        "SELECT prev_visit FROM user_state WHERE user_id=?1",
        [user_id],
        |r| r.get(0),
    )
    .optional()?
    .flatten())
}

/// Records activity at `now` (RFC 3339): a gap of more than `gap_secs` since the last
/// activity starts a new visit, and the previous one becomes the `newSinceLastVisit` baseline.
/// The first visit ever is its own baseline. Returns whether anything was written.
pub fn track_visit(c: &Connection, user_id: i64, now: i64, gap_secs: i64) -> ApiResult<bool> {
    let now_s = rfc3339_at(now);
    let row: Option<(Option<String>, Option<String>)> = c
        .query_row(
            "SELECT last_visit, prev_visit FROM user_state WHERE user_id=?1",
            [user_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (last, prev) = row.unwrap_or((None, None));
    let last_t = last.as_deref().and_then(crate::util::parse_rfc3339);
    let (new_last, new_prev) = match last_t {
        None => (now_s.clone(), prev.unwrap_or_else(|| now_s.clone())),
        Some(t) if now - t > gap_secs => (now_s.clone(), last.clone().unwrap_or_default()),
        // same visit: refresh at most every 5 minutes
        Some(t) if now - t > 300 => (now_s.clone(), prev.unwrap_or_else(|| now_s.clone())),
        // upgraded database: last activity known, no baseline yet
        Some(_) if prev.is_none() => {
            let l = last.clone().unwrap_or_default();
            (l.clone(), l)
        }
        Some(_) => return Ok(false),
    };
    c.execute(
        "INSERT INTO user_state(user_id, last_visit, prev_visit) VALUES (?1,?2,?3) \
         ON CONFLICT(user_id) DO UPDATE SET last_visit=excluded.last_visit, prev_visit=excluded.prev_visit",
        params![user_id, new_last, new_prev],
    )?;
    Ok(true)
}

/// Mails sent by `user_id` on `day`.
pub fn mail_count(c: &Connection, user_id: i64, day: &str) -> ApiResult<i64> {
    Ok(c.query_row(
        "SELECT count FROM mail_count WHERE user_id=?1 AND day=?2",
        params![user_id, day],
        |r| r.get(0),
    )
    .optional()?
    .unwrap_or(0))
}

/// Counts one mail of `user_id` on `day` (older days are dropped).
pub fn add_mail(c: &Connection, user_id: i64, day: &str) -> ApiResult<()> {
    c.execute("DELETE FROM mail_count WHERE day < ?1", [day])?;
    c.execute(
        "INSERT INTO mail_count(user_id, day, count) VALUES (?1,?2,1) \
         ON CONFLICT(user_id, day) DO UPDATE SET count = count + 1",
        params![user_id, day],
    )?;
    Ok(())
}

pub fn get_prefs(c: &Connection, user_id: i64) -> ApiResult<String> {
    Ok(c.query_row(
        "SELECT prefs FROM user_state WHERE user_id=?1",
        [user_id],
        |r| r.get(0),
    )
    .optional()?
    .unwrap_or_else(|| "{}".into()))
}

pub fn set_prefs(c: &Connection, user_id: i64, json: &str) -> ApiResult<()> {
    c.execute(
        "INSERT INTO user_state(user_id, prefs) VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET prefs=excluded.prefs",
        params![user_id, json],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- libraries

#[derive(Debug, Clone)]
pub struct LibraryRow {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub inpx: Option<String>,
    pub first_author_only: bool,
    pub skip_deleted: bool,
    pub is_default: bool,
}

const LIB_COLS: &str = "id, name, path, inpx, first_author_only, skip_deleted, is_default";

fn lib_row(r: &rusqlite::Row) -> rusqlite::Result<LibraryRow> {
    Ok(LibraryRow {
        id: r.get(0)?,
        name: r.get(1)?,
        path: r.get(2)?,
        inpx: r.get(3)?,
        first_author_only: r.get(4)?,
        skip_deleted: r.get(5)?,
        is_default: r.get(6)?,
    })
}

pub fn list_libraries(c: &Connection) -> ApiResult<Vec<LibraryRow>> {
    let mut st = c.prepare(&format!("SELECT {LIB_COLS} FROM library ORDER BY id"))?;
    let rows = st.query_map([], lib_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get_library(c: &Connection, id: i64) -> ApiResult<Option<LibraryRow>> {
    Ok(c.query_row(
        &format!("SELECT {LIB_COLS} FROM library WHERE id=?1"),
        [id],
        lib_row,
    )
    .optional()?)
}

pub fn insert_library(c: &Connection, l: &LibraryRow) -> ApiResult<i64> {
    if l.is_default {
        c.execute("UPDATE library SET is_default=0", [])?;
    }
    c.execute(
        "INSERT INTO library(name, path, inpx, first_author_only, skip_deleted, is_default) VALUES (?1,?2,?3,?4,?5,?6)",
        params![l.name, l.path, l.inpx, l.first_author_only, l.skip_deleted, l.is_default],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn update_library(c: &Connection, l: &LibraryRow) -> ApiResult<()> {
    if l.is_default {
        c.execute("UPDATE library SET is_default=0 WHERE id<>?1", [l.id])?;
    }
    c.execute(
        "UPDATE library SET name=?1, path=?2, inpx=?3, first_author_only=?4, skip_deleted=?5, is_default=?6 WHERE id=?7",
        params![l.name, l.path, l.inpx, l.first_author_only, l.skip_deleted, l.is_default, l.id],
    )?;
    Ok(())
}

pub fn delete_library(c: &Connection, id: i64) -> ApiResult<()> {
    c.execute("DELETE FROM shelf_book WHERE library_id=?1", [id])?;
    c.execute("DELETE FROM rating WHERE library_id=?1", [id])?;
    c.execute("DELETE FROM library WHERE id=?1", [id])?;
    Ok(())
}

// ---------------------------------------------------------------- ratings & shelves

/// book_key → rating, book_key → shelf ids.
pub type UserMarks = (HashMap<String, i64>, HashMap<String, Vec<i64>>);

/// Ratings and shelf ids of `user` for the given book keys of one library.
pub fn user_marks(
    c: &Connection,
    user_id: i64,
    lib_id: i64,
    keys: &[String],
) -> ApiResult<UserMarks> {
    let mut ratings = HashMap::new();
    let mut shelves: HashMap<String, Vec<i64>> = HashMap::new();
    if keys.is_empty() {
        return Ok((ratings, shelves));
    }
    let arr = text_array(keys);
    let mut st = c.prepare_cached(
        "SELECT book_key, rating FROM rating WHERE user_id=?1 AND library_id=?2 AND book_key IN rarray(?3)",
    )?;
    for r in st.query_map(params![user_id, lib_id, arr], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })? {
        let (k, v) = r?;
        ratings.insert(k, v);
    }
    let mut st = c.prepare_cached(
        "SELECT sb.book_key, sb.shelf_id FROM shelf_book sb JOIN shelf s ON s.id = sb.shelf_id \
         WHERE s.user_id=?1 AND sb.library_id=?2 AND sb.book_key IN rarray(?3) ORDER BY sb.shelf_id",
    )?;
    for r in st.query_map(params![user_id, lib_id, arr], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })? {
        let (k, v) = r?;
        shelves.entry(k).or_default().push(v);
    }
    Ok((ratings, shelves))
}

pub fn set_rating(
    c: &Connection,
    user_id: i64,
    lib_id: i64,
    key: &str,
    rating: i64,
) -> ApiResult<()> {
    if rating == 0 {
        c.execute(
            "DELETE FROM rating WHERE user_id=?1 AND library_id=?2 AND book_key=?3",
            params![user_id, lib_id, key],
        )?;
    } else {
        c.execute(
            "INSERT INTO rating(user_id, library_id, book_key, rating) VALUES (?1,?2,?3,?4) \
             ON CONFLICT(user_id, library_id, book_key) DO UPDATE SET rating=excluded.rating",
            params![user_id, lib_id, key, rating],
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Shelf {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub count: i64,
}

const SHELF_SELECT: &str = "SELECT s.id, s.name, s.color, (SELECT count(*) FROM shelf_book b WHERE b.shelf_id = s.id) FROM shelf s";

fn shelf_row(r: &rusqlite::Row) -> rusqlite::Result<Shelf> {
    Ok(Shelf {
        id: r.get(0)?,
        name: r.get(1)?,
        color: r.get(2)?,
        count: r.get(3)?,
    })
}

pub fn list_shelves(c: &Connection, user_id: i64) -> ApiResult<Vec<Shelf>> {
    let mut st = c.prepare(&format!(
        "{SHELF_SELECT} WHERE s.user_id=?1 ORDER BY s.name COLLATE NOCASE, s.id"
    ))?;
    let rows = st.query_map([user_id], shelf_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get_shelf(c: &Connection, user_id: i64, id: i64) -> ApiResult<Shelf> {
    c.query_row(
        &format!("{SHELF_SELECT} WHERE s.user_id=?1 AND s.id=?2"),
        params![user_id, id],
        shelf_row,
    )
    .optional()?
    .ok_or_else(|| ApiError::not_found("shelf not found"))
}

pub fn create_shelf(c: &Connection, user_id: i64, name: &str, color: &str) -> ApiResult<Shelf> {
    c.execute(
        "INSERT INTO shelf(user_id, name, color) VALUES (?1,?2,?3)",
        params![user_id, name, color],
    )?;
    get_shelf(c, user_id, c.last_insert_rowid())
}

pub fn update_shelf(
    c: &Connection,
    user_id: i64,
    id: i64,
    name: Option<&str>,
    color: Option<&str>,
) -> ApiResult<Shelf> {
    get_shelf(c, user_id, id)?;
    if let Some(n) = name {
        c.execute("UPDATE shelf SET name=?1 WHERE id=?2", params![n, id])?;
    }
    if let Some(col) = color {
        c.execute("UPDATE shelf SET color=?1 WHERE id=?2", params![col, id])?;
    }
    get_shelf(c, user_id, id)
}

pub fn delete_shelf(c: &Connection, user_id: i64, id: i64) -> ApiResult<()> {
    get_shelf(c, user_id, id)?;
    c.execute("DELETE FROM shelf_book WHERE shelf_id=?1", [id])?;
    c.execute("DELETE FROM shelf WHERE id=?1", [id])?;
    Ok(())
}

pub fn shelf_keys(c: &Connection, shelf_id: i64, lib_id: i64) -> ApiResult<Vec<String>> {
    let mut st =
        c.prepare("SELECT book_key FROM shelf_book WHERE shelf_id=?1 AND library_id=?2")?;
    let rows = st.query_map(params![shelf_id, lib_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn shelf_modify(
    c: &mut Connection,
    shelf_id: i64,
    lib_id: i64,
    keys: &[String],
    add: bool,
) -> ApiResult<()> {
    let tx = c.transaction()?;
    {
        let now = now_rfc3339();
        let mut ins = tx.prepare(
            "INSERT OR IGNORE INTO shelf_book(shelf_id, library_id, book_key, added_at) VALUES (?1,?2,?3,?4)",
        )?;
        let mut del = tx.prepare(
            "DELETE FROM shelf_book WHERE shelf_id=?1 AND library_id=?2 AND book_key=?3",
        )?;
        for k in keys {
            if add {
                ins.execute(params![shelf_id, lib_id, k, now])?;
            } else {
                del.execute(params![shelf_id, lib_id, k])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

// ---------------------------------------------------------------- devices

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub format: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default = "default_file_name")]
    pub file_name: String,
    #[serde(default)]
    pub shared: bool,
    #[serde(default)]
    pub options: ConvertOptions,
    #[serde(skip)]
    pub user_id: Option<i64>,
}

pub fn default_file_name() -> String {
    "%a - %s %n - %b".into()
}

fn device_row(r: &rusqlite::Row) -> rusqlite::Result<Device> {
    let user_id: Option<i64> = r.get(1)?;
    let options: String = r.get(7)?;
    Ok(Device {
        id: r.get(0)?,
        user_id,
        shared: user_id.is_none(),
        name: r.get(2)?,
        kind: r.get(3)?,
        format: r.get(4)?,
        target: r.get(5)?,
        file_name: r.get(6)?,
        options: serde_json::from_str(&options).unwrap_or_default(),
    })
}

const DEVICE_COLS: &str = "id, user_id, name, kind, format, target, file_name, options";

pub fn list_devices(c: &Connection, user_id: i64) -> ApiResult<Vec<Device>> {
    let mut st = c.prepare(&format!(
        "SELECT {DEVICE_COLS} FROM device WHERE user_id IS NULL OR user_id=?1 ORDER BY id"
    ))?;
    let rows = st.query_map([user_id], device_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A device visible to `user_id` (shared or own).
pub fn get_device(c: &Connection, user_id: i64, id: i64) -> ApiResult<Device> {
    c.query_row(
        &format!(
            "SELECT {DEVICE_COLS} FROM device WHERE id=?1 AND (user_id IS NULL OR user_id=?2)"
        ),
        params![id, user_id],
        device_row,
    )
    .optional()?
    .ok_or_else(|| ApiError::not_found("device not found"))
}

pub fn save_device(c: &Connection, d: &Device) -> ApiResult<i64> {
    let options =
        serde_json::to_string(&d.options).map_err(|e| ApiError::internal(e.to_string()))?;
    if d.id == 0 {
        c.execute(
            "INSERT INTO device(user_id, name, kind, format, target, file_name, options) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![d.user_id, d.name, d.kind, d.format, d.target, d.file_name, options],
        )?;
        Ok(c.last_insert_rowid())
    } else {
        c.execute(
            "UPDATE device SET user_id=?1, name=?2, kind=?3, format=?4, target=?5, file_name=?6, options=?7 WHERE id=?8",
            params![d.user_id, d.name, d.kind, d.format, d.target, d.file_name, options, d.id],
        )?;
        Ok(d.id)
    }
}

pub fn delete_device(c: &Connection, id: i64) -> ApiResult<()> {
    c.execute("DELETE FROM device WHERE id=?1", [id])?;
    Ok(())
}

/// Seeds the shared default devices once (API.md "Devices and sending").
pub fn seed_devices(c: &Connection) -> ApiResult<()> {
    if get_setting_raw(c, "devices_seeded")?.is_some() {
        return Ok(());
    }
    let defaults: [(&str, &str, &str, Option<&str>); 6] = [
        ("Kindle", "email", "epub", None),
        ("Kindle (USB)", "download", "azw3", None),
        ("Apple Books", "download", "epub", None),
        ("Kobo", "download", "kepub", None),
        ("Server folder", "folder", "epub", Some("")),
        ("Original", "download", "original", None),
    ];
    for (name, kind, format, target) in defaults {
        let d = Device {
            id: 0,
            name: name.into(),
            kind: kind.into(),
            format: format.into(),
            target: target.map(String::from),
            file_name: default_file_name(),
            shared: true,
            options: ConvertOptions::default(),
            user_id: None,
        };
        save_device(c, &d)?;
    }
    put_setting_raw(c, "devices_seeded", "true")?;
    Ok(())
}

// ---------------------------------------------------------------- settings

pub fn get_setting_raw(c: &Connection, key: &str) -> ApiResult<Option<String>> {
    Ok(
        c.query_row("SELECT value FROM setting WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()?,
    )
}

pub fn put_setting_raw(c: &Connection, key: &str, value: &str) -> ApiResult<()> {
    c.execute(
        "INSERT INTO setting(key, value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn get_setting<T: DeserializeOwned + Default>(c: &Connection, key: &str) -> ApiResult<T> {
    Ok(get_setting_raw(c, key)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

pub fn put_setting<T: Serialize>(c: &Connection, key: &str, v: &T) -> ApiResult<()> {
    let s = serde_json::to_string(v).map_err(|e| ApiError::internal(e.to_string()))?;
    put_setting_raw(c, key, &s)
}

/// Stored SMTP settings (the password never leaves the server).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: String,
    pub username: String,
    pub from: String,
    pub password: Option<String>,
    pub pause_seconds: u64,
    /// Recipient patterns (`*` = any characters, case-insensitive) that `/send` and e-mail
    /// devices may use; a lone `*` allows every address.
    pub allowed_recipients: Vec<String>,
    /// Mails per user and day (server local date).
    pub daily_limit_per_user: u32,
    /// Subject of Send to Kindle mails; `%b` = book title, `%a` = author(s).
    pub subject: String,
}

pub fn default_allowed_recipients() -> Vec<String> {
    vec!["*@kindle.com".into(), "*@free.kindle.com".into()]
}

impl Default for SmtpConfig {
    fn default() -> Self {
        SmtpConfig {
            host: String::new(),
            port: 587,
            security: "starttls".into(),
            username: String::new(),
            from: String::new(),
            password: None,
            pause_seconds: 2,
            allowed_recipients: default_allowed_recipients(),
            daily_limit_per_user: 100,
            subject: "%b".into(),
        }
    }
}

impl SmtpConfig {
    /// The mail subject for a book, from the [`subject`](Self::subject) template.
    pub fn subject_for(&self, title: &str, authors: &str) -> String {
        let tpl = if self.subject.trim().is_empty() {
            "%b"
        } else {
            self.subject.trim()
        };
        let s = tpl.replace("%b", title).replace("%a", authors);
        // a header value: no line breaks, bounded length
        let s: String = s.chars().filter(|c| !c.is_control()).take(250).collect();
        if s.trim().is_empty() {
            title.to_string()
        } else {
            s
        }
    }

    /// Whether `addr` matches one of [`allowed_recipients`](Self::allowed_recipients).
    pub fn recipient_allowed(&self, addr: &str) -> bool {
        let a = addr.trim().to_lowercase();
        self.allowed_recipients
            .iter()
            .any(|p| glob_match(&p.trim().to_lowercase(), &a))
    }
}

/// `*` matches any run of characters (including none); everything else literally.
pub fn glob_match(pattern: &str, s: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipients() {
        let s = SmtpConfig::default();
        assert!(s.recipient_allowed("Me@Kindle.com"));
        assert!(s.recipient_allowed("x_1@free.kindle.com"));
        assert!(!s.recipient_allowed("me@kindle.com.evil.org"));
        assert!(!s.recipient_allowed("victim@example.com"));
        assert!(!s.recipient_allowed("me@evilkindle.com"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*b*c", "aXbYc"));
        assert!(!glob_match("a*b", "aXbYc"));
    }

    #[test]
    fn visits() {
        let c = freelib_catalog::open_app_db(std::path::Path::new(":memory:")).unwrap();
        let t0 = 1_700_000_000;
        assert!(track_visit(&c, 1, t0, 1800).unwrap());
        assert_eq!(prev_visit(&c, 1).unwrap(), Some(rfc3339_at(t0)));
        // same visit: no new baseline
        track_visit(&c, 1, t0 + 600, 1800).unwrap();
        assert_eq!(prev_visit(&c, 1).unwrap(), Some(rfc3339_at(t0)));
        // a day later: the previous visit's last activity becomes the baseline
        track_visit(&c, 1, t0 + 86_400, 1800).unwrap();
        assert_eq!(prev_visit(&c, 1).unwrap(), Some(rfc3339_at(t0 + 600)));
        assert_eq!(last_visit(&c, 1).unwrap(), Some(rfc3339_at(t0 + 86_400)));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpdsConfig {
    pub enabled: bool,
    pub require_auth: bool,
}

impl Default for OpdsConfig {
    fn default() -> Self {
        OpdsConfig {
            enabled: true,
            require_auth: true,
        }
    }
}

#[cfg(test)]
mod smtp_subject_tests {
    use super::SmtpConfig;

    #[test]
    fn subject_template() {
        let mut c = SmtpConfig::default();
        assert_eq!(c.subject_for("Dune", "Frank Herbert"), "Dune");
        c.subject = "freeLib - %b (%a)".into();
        assert_eq!(
            c.subject_for("Dune", "Frank Herbert"),
            "freeLib - Dune (Frank Herbert)"
        );
        c.subject = "static\r\nBcc: x@y".into();
        assert_eq!(c.subject_for("Dune", ""), "staticBcc: x@y");
        c.subject = "   ".into();
        assert_eq!(c.subject_for("Dune", ""), "Dune");
    }
}
