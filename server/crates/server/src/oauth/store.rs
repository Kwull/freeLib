//! `app.db` tables of the OAuth authorization server: `oauth_client` (dynamically registered
//! clients), `oauth_grant` (one row per authorization: an "authorized app" of a user) and
//! `oauth_token` (access and refresh tokens, by the SHA-256 of their secret only).

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::db::{self, User};
use crate::error::ApiResult;
use crate::util::{now_rfc3339, rfc3339_at};

/// A dynamically registered client (RFC 7591).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcrClient {
    pub client_id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub client_uri: Option<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

pub fn insert_client(c: &Connection, cl: &DcrClient) -> ApiResult<()> {
    c.execute(
        "INSERT INTO oauth_client(client_id, name, redirect_uris, client_uri, created_at) VALUES (?1,?2,?3,?4,?5)",
        params![
            cl.client_id,
            cl.name,
            serde_json::to_string(&cl.redirect_uris).unwrap_or_else(|_| "[]".into()),
            cl.client_uri,
            cl.created_at
        ],
    )?;
    Ok(())
}

pub fn count_clients(c: &Connection) -> ApiResult<i64> {
    Ok(c.query_row("SELECT count(*) FROM oauth_client", [], |r| r.get(0))?)
}

pub fn get_client(c: &Connection, id: &str) -> ApiResult<Option<DcrClient>> {
    Ok(c.query_row(
        "SELECT client_id, name, redirect_uris, client_uri, created_at, last_used_at FROM oauth_client WHERE client_id=?1",
        [id],
        |r| {
            let uris: String = r.get(2)?;
            Ok(DcrClient {
                client_id: r.get(0)?,
                name: r.get(1)?,
                redirect_uris: serde_json::from_str(&uris).unwrap_or_default(),
                client_uri: r.get(3)?,
                created_at: r.get(4)?,
                last_used_at: r.get(5)?,
            })
        },
    )
    .optional()?)
}

/// Records that a client was used for an authorization (it is then kept longer).
pub fn touch_client(c: &Connection, id: &str, now: i64) -> ApiResult<()> {
    c.execute(
        "UPDATE oauth_client SET last_used_at=?2 WHERE client_id=?1",
        params![id, now],
    )?;
    Ok(())
}

/// Removes registered clients that were never used for `unused_secs`, and clients without
/// grants that were last used more than `idle_secs` ago. Returns how many were removed.
pub fn delete_stale_clients(
    c: &Connection,
    now: i64,
    unused_secs: i64,
    idle_secs: i64,
) -> ApiResult<usize> {
    let a = c.execute(
        "DELETE FROM oauth_client WHERE last_used_at IS NULL AND created_at < ?1",
        [now - unused_secs],
    )?;
    let b = c.execute(
        "DELETE FROM oauth_client WHERE last_used_at IS NOT NULL AND last_used_at < ?1 \
         AND NOT EXISTS (SELECT 1 FROM oauth_grant g WHERE g.client_id = oauth_client.client_id)",
        [now - idle_secs],
    )?;
    Ok(a + b)
}

/// What a new grant records.
pub struct NewGrant<'a> {
    pub user_id: i64,
    pub client_id: &'a str,
    pub client_name: &'a str,
    pub client_kind: &'a str,
    pub redirect_uri: &'a str,
    pub scopes: &'a [String],
    pub resource: &'a str,
}

/// Grants kept per user; the least recently used ones go first.
pub const MAX_GRANTS_PER_USER: i64 = 50;

/// Creates a grant with its first access and refresh token; returns the grant id.
pub fn create_grant(
    c: &Connection,
    g: &NewGrant,
    access: (&str, i64),
    refresh: (&str, i64),
) -> ApiResult<i64> {
    let tx = c.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO oauth_grant(user_id, client_id, client_name, client_kind, redirect_uri, scopes, resource, created_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            g.user_id,
            g.client_id,
            g.client_name,
            g.client_kind,
            g.redirect_uri,
            g.scopes.join(" "),
            g.resource,
            now_rfc3339()
        ],
    )?;
    let id = tx.last_insert_rowid();
    let scopes = g.scopes.join(" ");
    insert_token(&tx, access.0, id, "access", &scopes, access.1)?;
    insert_token(&tx, refresh.0, id, "refresh", &scopes, refresh.1)?;
    tx.execute(
        "DELETE FROM oauth_grant WHERE user_id=?1 AND id IN (SELECT id FROM oauth_grant WHERE user_id=?1 \
         ORDER BY coalesce(last_used_at, created_at) DESC, id DESC LIMIT -1 OFFSET ?2)",
        params![g.user_id, MAX_GRANTS_PER_USER],
    )?;
    tx.commit()?;
    Ok(id)
}

fn insert_token(
    c: &Connection,
    hash: &str,
    grant_id: i64,
    kind: &str,
    scopes: &str,
    expires_at: i64,
) -> ApiResult<()> {
    c.execute(
        "INSERT INTO oauth_token(token_hash, grant_id, kind, scopes, expires_at) VALUES (?1,?2,?3,?4,?5)",
        params![hash, grant_id, kind, scopes, expires_at],
    )?;
    Ok(())
}

fn split_scopes(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

/// A valid access token.
#[derive(Debug, Clone)]
pub struct AccessRow {
    pub grant_id: i64,
    pub client_name: String,
    pub scopes: Vec<String>,
    pub resource: String,
    pub expires_at: i64,
}

/// The unexpired access token with this hash, and its user (`None` for a deleted user; user 0
/// is the open-mode admin).
pub fn access_token(c: &Connection, hash: &str, now: i64) -> ApiResult<Option<(AccessRow, User)>> {
    let row: Option<(AccessRow, i64)> = c
        .query_row(
            "SELECT t.grant_id, g.client_name, t.scopes, g.resource, t.expires_at, g.user_id \
             FROM oauth_token t JOIN oauth_grant g ON g.id = t.grant_id \
             WHERE t.token_hash=?1 AND t.kind='access' AND t.expires_at > ?2",
            params![hash, now],
            |r| {
                let scopes: String = r.get(2)?;
                Ok((
                    AccessRow {
                        grant_id: r.get(0)?,
                        client_name: r.get(1)?,
                        scopes: split_scopes(&scopes),
                        resource: r.get(3)?,
                        expires_at: r.get(4)?,
                    },
                    r.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((row, uid)) = row else {
        return Ok(None);
    };
    let user = if uid == 0 {
        Some(User::open_mode_admin())
    } else {
        db::get_user(c, uid)?
    };
    Ok(user.map(|u| (row, u)))
}

/// Records a use of the grant (at most once a minute).
pub fn touch_grant(c: &Connection, id: i64, now: i64) -> ApiResult<()> {
    c.execute(
        "UPDATE oauth_grant SET last_used_at=?2 WHERE id=?1 AND (last_used_at IS NULL OR last_used_at < ?3)",
        params![id, rfc3339_at(now), rfc3339_at(now - 60)],
    )?;
    Ok(())
}

/// A refresh token (valid, rotated or expired) and its grant.
#[derive(Debug, Clone)]
pub struct RefreshRow {
    pub grant_id: i64,
    pub user_id: i64,
    pub client_id: String,
    pub client_name: String,
    /// The grant's scopes.
    pub scopes: Vec<String>,
    pub resource: String,
    pub expires_at: i64,
    pub rotated_at: Option<i64>,
}

pub fn refresh_token(c: &Connection, hash: &str) -> ApiResult<Option<RefreshRow>> {
    Ok(c.query_row(
        "SELECT t.grant_id, g.user_id, g.client_id, g.client_name, g.scopes, g.resource, t.expires_at, t.rotated_at \
         FROM oauth_token t JOIN oauth_grant g ON g.id = t.grant_id WHERE t.token_hash=?1 AND t.kind='refresh'",
        [hash],
        |r| {
            let scopes: String = r.get(4)?;
            Ok(RefreshRow {
                grant_id: r.get(0)?,
                user_id: r.get(1)?,
                client_id: r.get(2)?,
                client_name: r.get(3)?,
                scopes: split_scopes(&scopes),
                resource: r.get(5)?,
                expires_at: r.get(6)?,
                rotated_at: r.get(7)?,
            })
        },
    )
    .optional()?)
}

/// Rotates refresh token `old` (marks it used, only if it was not used yet) and stores the
/// new access and refresh tokens. `false` when `old` was rotated concurrently.
pub fn rotate(
    c: &Connection,
    old: &str,
    grant_id: i64,
    now: i64,
    access: (&str, i64, &str),
    refresh: (&str, i64),
) -> ApiResult<bool> {
    let tx = c.unchecked_transaction()?;
    let n = tx.execute(
        "UPDATE oauth_token SET rotated_at=?2 WHERE token_hash=?1 AND kind='refresh' AND rotated_at IS NULL",
        params![old, now],
    )?;
    if n == 0 {
        return Ok(false);
    }
    let grant_scopes: String = tx.query_row(
        "SELECT scopes FROM oauth_grant WHERE id=?1",
        [grant_id],
        |r| r.get(0),
    )?;
    insert_token(&tx, access.0, grant_id, "access", access.2, access.1)?;
    insert_token(
        &tx,
        refresh.0,
        grant_id,
        "refresh",
        &grant_scopes,
        refresh.1,
    )?;
    tx.execute(
        "UPDATE oauth_grant SET last_used_at=?2 WHERE id=?1",
        params![grant_id, rfc3339_at(now)],
    )?;
    tx.commit()?;
    Ok(true)
}

/// Deletes a grant and all its tokens; whether it existed.
pub fn revoke_grant(c: &Connection, id: i64) -> ApiResult<bool> {
    Ok(c.execute("DELETE FROM oauth_grant WHERE id=?1", [id])? > 0)
}

/// Deletes grant `id` of `user_id`; its client name when it existed.
pub fn revoke_user_grant(c: &Connection, user_id: i64, id: i64) -> ApiResult<Option<String>> {
    let name: Option<String> = c
        .query_row(
            "SELECT client_name FROM oauth_grant WHERE id=?1 AND user_id=?2",
            params![id, user_id],
            |r| r.get(0),
        )
        .optional()?;
    if name.is_some() {
        revoke_grant(c, id)?;
    }
    Ok(name)
}

/// All grants of a user go (user deleted, or data taken over from open mode).
pub fn delete_user_grants(c: &Connection, user_id: i64) -> ApiResult<()> {
    c.execute("DELETE FROM oauth_grant WHERE user_id=?1", [user_id])?;
    Ok(())
}

/// Token revocation (RFC 7009): a refresh token revokes its whole grant, an access token only
/// itself. With `client_id`, only tokens of that client are revoked. Returns the grant and its
/// user when something was revoked.
pub fn revoke_token(
    c: &Connection,
    hash: &str,
    client_id: Option<&str>,
) -> ApiResult<Option<(i64, i64, String)>> {
    let row: Option<(i64, String, i64, String, String)> = c
        .query_row(
            "SELECT t.grant_id, t.kind, g.user_id, g.client_id, g.client_name FROM oauth_token t \
             JOIN oauth_grant g ON g.id = t.grant_id WHERE t.token_hash=?1",
            [hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((grant, kind, uid, cid, name)) = row else {
        return Ok(None);
    };
    if client_id.is_some_and(|x| x != cid) {
        return Ok(None);
    }
    if kind == "refresh" {
        revoke_grant(c, grant)?;
    } else {
        c.execute("DELETE FROM oauth_token WHERE token_hash=?1", [hash])?;
    }
    Ok(Some((grant, uid, name)))
}

/// An authorized app as listed in Settings → Account.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantRow {
    pub id: i64,
    pub client_name: String,
    /// `cimd` (the client is identified by its https URL) or `dcr` (registered here).
    pub client_kind: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

pub fn list_grants(c: &Connection, user_id: i64) -> ApiResult<Vec<GrantRow>> {
    let mut st = c.prepare(
        "SELECT id, client_name, client_kind, client_id, redirect_uri, scopes, created_at, last_used_at \
         FROM oauth_grant WHERE user_id=?1 ORDER BY coalesce(last_used_at, created_at) DESC, id DESC",
    )?;
    let rows = st.query_map([user_id], |r| {
        let scopes: String = r.get(5)?;
        Ok(GrantRow {
            id: r.get(0)?,
            client_name: r.get(1)?,
            client_kind: r.get(2)?,
            client_id: r.get(3)?,
            redirect_uri: r.get(4)?,
            scopes: split_scopes(&scopes),
            created_at: r.get(6)?,
            last_used_at: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Removes expired tokens, then grants without any token left. Returns (tokens, grants).
pub fn cleanup(c: &Connection, now: i64) -> ApiResult<(usize, usize)> {
    let t = c.execute("DELETE FROM oauth_token WHERE expires_at <= ?1", [now])?;
    let g = c.execute(
        "DELETE FROM oauth_grant WHERE NOT EXISTS (SELECT 1 FROM oauth_token t WHERE t.grant_id = oauth_grant.id)",
        [],
    )?;
    Ok((t, g))
}
