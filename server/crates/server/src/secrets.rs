//! Encryption at rest for reversible secrets (the SMTP password, and any future secret the
//! server must be able to read back). Login passwords are not here: they are stored as Argon2id
//! hashes, and session / API / OAuth tokens as SHA-256 hashes, because the server only ever has
//! to *compare* them, never recover them.
//!
//! * **Cipher**: XChaCha20-Poly1305 (AEAD) with a random 192-bit nonce per value. The associated
//!   data binds a value to its name (`smtp.password`), so a ciphertext copied into another
//!   field fails to decrypt.
//! * **Format**: `enc:v1:<key id>:<base64url(nonce ‖ ciphertext ‖ tag)>`. The key id is the
//!   first 4 bytes (hex) of a SHA-256 over the key: it tells which key encrypted a value
//!   without revealing anything about the key.
//! * **Key**: `FREELIB_SECRET_KEY` (32 bytes as 64 hex digits or base64), else the file named by
//!   `FREELIB_SECRET_KEY_FILE` (for Docker secrets), else `<data dir>/secret.key`, created on
//!   the first start with mode 0600.
//! * **Rotation**: with `FREELIB_SECRET_KEY_OLD`, values encrypted with the old key are
//!   re-encrypted with the current one at start ([`migrate`]).
//! * **Wrong key**: when app.db holds values encrypted with a key that is not configured, the
//!   server refuses to start ([`migrate`] fails) instead of silently losing them.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::config::Config;
use crate::db;
use crate::error::{ApiError, ApiResult};

/// Prefix of every encrypted value.
pub const PREFIX: &str = "enc:";
const VERSION: &str = "v1";
const NONCE_LEN: usize = 24;
/// Name of the generated key file in the data directory.
pub const KEY_FILE: &str = "secret.key";

/// Secret fields of JSON settings: (setting key, JSON field, name bound into the ciphertext).
/// Every reversible secret the server stores must be listed here, so that start-up encrypts
/// legacy plain-text values, re-encrypts on key rotation and refuses a wrong key.
pub const SECRET_FIELDS: &[(&str, &str, &str)] = &[("smtp", "password", "smtp.password")];

/// A string whose `Debug` output does not show it (configuration values that are secrets).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Redacted(pub String);

impl From<&str> for Redacted {
    fn from(s: &str) -> Redacted {
        Redacted(s.to_string())
    }
}

impl From<String> for Redacted {
    fn from(s: String) -> Redacted {
        Redacted(s)
    }
}

impl std::ops::Deref for Redacted {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"<redacted>\"")
    }
}

/// Where the current key came from (for the start-up log line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    Env,
    File(PathBuf),
    Generated(PathBuf),
}

impl fmt::Display for KeySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeySource::Env => f.write_str("FREELIB_SECRET_KEY"),
            KeySource::File(p) => write!(f, "{}", p.display()),
            KeySource::Generated(p) => write!(f, "{} (generated)", p.display()),
        }
    }
}

struct Key {
    cipher: XChaCha20Poly1305,
    id: String,
}

impl Key {
    fn new(bytes: &[u8; 32]) -> Key {
        let mut h = Sha256::new();
        h.update(b"freelib secret key id\0");
        h.update(bytes);
        let id = hex::encode(&h.finalize()[..4]);
        Key {
            cipher: XChaCha20Poly1305::new(bytes.into()),
            id,
        }
    }
}

/// Why a stored value cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// Not in the `enc:v1:…` format.
    Malformed,
    /// Encrypted with a key that is not configured (its id).
    UnknownKey(String),
    /// The key id matches but authentication failed (tampered, or moved to another field).
    Corrupt,
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretError::Malformed => f.write_str("malformed encrypted value"),
            SecretError::UnknownKey(id) => write!(f, "encrypted with unknown key {id}"),
            SecretError::Corrupt => f.write_str("encrypted value fails authentication"),
        }
    }
}

/// The encryption keys of this server.
pub struct Secrets {
    current: Key,
    old: Option<Key>,
    pub source: KeySource,
}

impl fmt::Debug for Secrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secrets")
            .field("key_id", &self.current.id)
            .field("old_key_id", &self.old.as_ref().map(|k| &k.id))
            .field("source", &self.source)
            .finish()
    }
}

/// Parses a key: 64 hex digits, or base64 (standard or URL-safe, padding optional) of 32 bytes.
pub fn parse_key(s: &str) -> Result<Zeroizing<[u8; 32]>, String> {
    let s = s.trim();
    let bytes: Zeroizing<Vec<u8>> = if s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()) {
        Zeroizing::new(hex::decode(s).map_err(|_| "invalid hex")?)
    } else {
        let t = s.trim_end_matches('=');
        let d = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(t)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(t))
            .map_err(|_| "not 64 hex digits or base64".to_string())?;
        Zeroizing::new(d)
    };
    if bytes.len() != 32 {
        return Err(format!(
            "must be 32 bytes (64 hex digits or 44 base64 characters), got {} bytes",
            bytes.len()
        ));
    }
    let mut k = Zeroizing::new([0u8; 32]);
    k.copy_from_slice(&bytes);
    Ok(k)
}

/// A new random key, base64 (what `openssl rand -base64 32` prints).
pub fn generate_key_text() -> String {
    let mut b = Zeroizing::new([0u8; 32]);
    rand::fill(&mut b[..]);
    base64::engine::general_purpose::STANDARD.encode(&b[..])
}

fn read_key_file(p: &Path) -> Result<Zeroizing<[u8; 32]>, String> {
    let raw = Zeroizing::new(std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?);
    // a raw 32-byte file, or text
    if raw.len() == 32 && std::str::from_utf8(&raw).map_or(true, |s| parse_key(s).is_err()) {
        let mut k = Zeroizing::new([0u8; 32]);
        k.copy_from_slice(&raw);
        return Ok(k);
    }
    let text = std::str::from_utf8(&raw).map_err(|_| format!("{}: not a key", p.display()))?;
    parse_key(text).map_err(|e| format!("{}: {e}", p.display()))
}

/// Creates `path` with a new key, readable by this user only.
fn create_key_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let text = Zeroizing::new(generate_key_text());
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    f.write_all(format!("{}\n", text.as_str()).as_bytes())
        .and_then(|_| f.sync_all())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(())
}

/// Makes an existing key file private to its owner (it may come from a backup or `cp`).
fn tighten(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(m) = std::fs::metadata(path)
        && m.permissions().mode() & 0o077 != 0
    {
        match std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            Ok(()) => tracing::warn!(
                "{} was readable by other users; permissions set to 0600",
                path.display()
            ),
            Err(e) => tracing::warn!(
                "{} is readable by other users and its mode cannot be changed: {e}",
                path.display()
            ),
        }
    }
}

impl Secrets {
    /// A key set from raw keys (tests, tools).
    pub fn from_keys(current: &[u8; 32], old: Option<&[u8; 32]>) -> Secrets {
        Secrets {
            current: Key::new(current),
            old: old.map(Key::new),
            source: KeySource::Env,
        }
    }

    /// Loads the keys: `FREELIB_SECRET_KEY`, else `FREELIB_SECRET_KEY_FILE`, else
    /// `<data dir>/secret.key` (generated when missing). `FREELIB_SECRET_KEY_OLD` is the key
    /// being rotated away from.
    pub fn load(cfg: &Config) -> anyhow::Result<Secrets> {
        let (key, source) = if let Some(k) = &cfg.secret_key {
            let k = parse_key(&k.0).map_err(|e| anyhow::anyhow!("FREELIB_SECRET_KEY {e}"))?;
            (k, KeySource::Env)
        } else if let Some(p) = &cfg.secret_key_file {
            let k = read_key_file(p).map_err(|e| anyhow::anyhow!("FREELIB_SECRET_KEY_FILE {e}"))?;
            (k, KeySource::File(p.clone()))
        } else {
            let p = cfg.data_dir.join(KEY_FILE);
            if p.exists() {
                tighten(&p);
                let k = read_key_file(&p).map_err(|e| anyhow::anyhow!("secret key {e}"))?;
                (k, KeySource::File(p))
            } else {
                create_key_file(&p).map_err(|e| anyhow::anyhow!("secret key: {e}"))?;
                let k = read_key_file(&p).map_err(|e| anyhow::anyhow!("secret key {e}"))?;
                tracing::warn!(
                    "generated a new encryption key for stored secrets at {}: BACK IT UP together \
                     with app.db (without it the SMTP password cannot be read), or set \
                     FREELIB_SECRET_KEY / FREELIB_SECRET_KEY_FILE to keep it outside the data volume",
                    p.display()
                );
                (k, KeySource::Generated(p))
            }
        };
        let old = match &cfg.secret_key_old {
            Some(o) => {
                Some(parse_key(&o.0).map_err(|e| anyhow::anyhow!("FREELIB_SECRET_KEY_OLD {e}"))?)
            }
            None => None,
        };
        let s = Secrets {
            current: Key::new(&key),
            old: old.as_ref().map(|k| Key::new(k)),
            source,
        };
        if s.old.as_ref().is_some_and(|o| o.id == s.current.id) {
            anyhow::bail!("FREELIB_SECRET_KEY_OLD is the same key as the current one");
        }
        Ok(s)
    }

    /// Id of the current key (safe to log).
    pub fn key_id(&self) -> &str {
        &self.current.id
    }

    fn aad(name: &str) -> Vec<u8> {
        format!("freelib/{VERSION}/{name}").into_bytes()
    }

    /// Encrypts `plain` for the field `name` with the current key.
    pub fn encrypt(&self, name: &str, plain: &str) -> String {
        let mut nonce = [0u8; NONCE_LEN];
        rand::fill(&mut nonce[..]);
        let aad = Self::aad(name);
        let ct = self
            .current
            .cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plain.as_bytes(),
                    aad: &aad,
                },
            )
            .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory input");
        let mut blob = Vec::with_capacity(NONCE_LEN + ct.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        format!(
            "{PREFIX}{VERSION}:{}:{}",
            self.current.id,
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(blob)
        )
    }

    /// Whether `s` is in the encrypted format (it may still use an unknown key).
    pub fn is_encrypted(s: &str) -> bool {
        s.starts_with(PREFIX)
    }

    /// The key id of an encrypted value.
    pub fn key_id_of(stored: &str) -> Option<&str> {
        let rest = stored.strip_prefix(PREFIX)?.strip_prefix(VERSION)?;
        rest.strip_prefix(':')?.split(':').next()
    }

    /// Decrypts a value of field `name` (current key, or the old one during a rotation).
    pub fn decrypt(&self, name: &str, stored: &str) -> Result<Zeroizing<String>, SecretError> {
        let rest = stored
            .strip_prefix(PREFIX)
            .and_then(|r| r.strip_prefix(VERSION))
            .and_then(|r| r.strip_prefix(':'))
            .ok_or(SecretError::Malformed)?;
        let (kid, b64) = rest.split_once(':').ok_or(SecretError::Malformed)?;
        let key = if kid == self.current.id {
            &self.current
        } else if let Some(o) = self.old.as_ref().filter(|o| o.id == kid) {
            o
        } else {
            return Err(SecretError::UnknownKey(kid.to_string()));
        };
        let blob = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(b64)
            .map_err(|_| SecretError::Malformed)?;
        if blob.len() < NONCE_LEN + 16 {
            return Err(SecretError::Malformed);
        }
        let (nonce, ct) = blob.split_at(NONCE_LEN);
        let aad = Self::aad(name);
        let plain = Zeroizing::new(
            key.cipher
                .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: &aad })
                .map_err(|_| SecretError::Corrupt)?,
        );
        String::from_utf8(plain.to_vec())
            .map(Zeroizing::new)
            .map_err(|_| SecretError::Corrupt)
    }

    /// Decrypts a stored value for use; a legacy plain-text value (not yet migrated) is
    /// returned as is.
    pub fn reveal(&self, name: &str, stored: &str) -> ApiResult<Zeroizing<String>> {
        if !Self::is_encrypted(stored) {
            return Ok(Zeroizing::new(stored.to_string()));
        }
        self.decrypt(name, stored).map_err(|e| {
            tracing::error!("stored secret {name} cannot be decrypted: {e}");
            ApiError::internal(format!("the stored {name} cannot be decrypted ({e})"))
        })
    }
}

/// What [`migrate`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrateReport {
    /// Plain-text values encrypted.
    pub encrypted: usize,
    /// Values re-encrypted from the old key.
    pub rotated: usize,
    /// Values already encrypted with the current key.
    pub current: usize,
}

/// At start: encrypts plain-text secrets in app.db, re-encrypts values of the old key with the
/// current one, and fails when a value was encrypted with a key that is not configured (or
/// fails authentication). Idempotent; one transaction.
pub fn migrate(c: &Connection, s: &Secrets) -> anyhow::Result<MigrateReport> {
    let tx = c.unchecked_transaction()?;
    let mut rep = MigrateReport::default();
    for (key, field, name) in SECRET_FIELDS {
        let Some(raw) = db::get_setting_raw(&tx, key).map_err(|e| anyhow::anyhow!("{e}"))? else {
            continue;
        };
        let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let Some(obj) = v.as_object_mut() else {
            continue;
        };
        let Some(stored) = obj.get(*field).and_then(|x| x.as_str()).map(str::to_string) else {
            continue;
        };
        if stored.is_empty() {
            continue;
        }
        let new = if !Secrets::is_encrypted(&stored) {
            rep.encrypted += 1;
            s.encrypt(name, &stored)
        } else {
            match s.decrypt(name, &stored) {
                Ok(plain) => {
                    if Secrets::key_id_of(&stored) == Some(s.key_id()) {
                        rep.current += 1;
                        continue;
                    }
                    rep.rotated += 1;
                    s.encrypt(name, &plain)
                }
                Err(SecretError::UnknownKey(kid)) => anyhow::bail!(
                    "app.db holds a secret ({name}) encrypted with key {kid}, but the configured \
                     key ({}) is {}. Start with the key that encrypted it (restore {KEY_FILE} \
                     from the backup that belongs to this app.db, or set FREELIB_SECRET_KEY), or \
                     pass that key as FREELIB_SECRET_KEY_OLD to re-encrypt with the new one. To \
                     discard the stored secrets instead (the SMTP password must then be entered \
                     again), run `freelib-server forget-secrets`.",
                    s.source,
                    s.key_id()
                ),
                Err(e) => anyhow::bail!(
                    "app.db holds a secret ({name}) that cannot be decrypted: {e}. Run \
                     `freelib-server forget-secrets` to discard the stored secrets."
                ),
            }
        };
        obj.insert(field.to_string(), serde_json::Value::String(new));
        db::put_setting_raw(&tx, key, &v.to_string()).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    tx.commit()?;
    Ok(rep)
}

/// Removes every stored secret (`freelib-server forget-secrets`): the way out when the key is
/// lost. Returns how many were removed.
pub fn forget(c: &Connection) -> anyhow::Result<usize> {
    let mut n = 0;
    for (key, field, _) in SECRET_FIELDS {
        let Some(raw) = db::get_setting_raw(c, key).map_err(|e| anyhow::anyhow!("{e}"))? else {
            continue;
        };
        let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if let Some(obj) = v.as_object_mut()
            && obj.remove(*field).is_some_and(|x| !x.is_null())
        {
            n += 1;
            db::put_setting_raw(c, key, &v.to_string()).map_err(|e| anyhow::anyhow!("{e}"))?;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> ([u8; 32], [u8; 32]) {
        ([7u8; 32], [9u8; 32])
    }

    #[test]
    fn round_trip_and_format() {
        let (a, _) = keys();
        let s = Secrets::from_keys(&a, None);
        let e1 = s.encrypt("smtp.password", "hunter2");
        let e2 = s.encrypt("smtp.password", "hunter2");
        assert_ne!(e1, e2, "random nonce per value");
        assert!(e1.starts_with(&format!("enc:v1:{}:", s.key_id())), "{e1}");
        assert!(!e1.contains("hunter2"));
        assert_eq!(s.decrypt("smtp.password", &e1).unwrap().as_str(), "hunter2");
        // bound to the field name
        assert_eq!(
            s.decrypt("other.secret", &e1).unwrap_err(),
            SecretError::Corrupt
        );
        // tampering
        let mut t = e1.clone().into_bytes();
        let last = t.len() - 3;
        t[last] = if t[last] == b'A' { b'B' } else { b'A' };
        let t = String::from_utf8(t).unwrap();
        assert!(s.decrypt("smtp.password", &t).is_err());
        assert_eq!(
            s.decrypt("smtp.password", "hunter2").unwrap_err(),
            SecretError::Malformed
        );
        assert_eq!(
            s.reveal("smtp.password", "legacy").unwrap().as_str(),
            "legacy"
        );
    }

    #[test]
    fn wrong_key_and_rotation() {
        let (a, b) = keys();
        let old = Secrets::from_keys(&a, None);
        let e = old.encrypt("smtp.password", "pw");
        let new = Secrets::from_keys(&b, None);
        assert_eq!(
            new.decrypt("smtp.password", &e).unwrap_err(),
            SecretError::UnknownKey(old.key_id().to_string())
        );
        let rot = Secrets::from_keys(&b, Some(&a));
        assert_eq!(rot.decrypt("smtp.password", &e).unwrap().as_str(), "pw");
    }

    #[test]
    fn key_parsing() {
        let hex64 = "00".repeat(32);
        assert!(parse_key(&hex64).is_ok());
        let b64 = generate_key_text();
        assert_eq!(b64.len(), 44);
        assert!(parse_key(&b64).is_ok());
        assert!(parse_key(b64.trim_end_matches('=')).is_ok());
        assert!(parse_key(&b64.replace('+', "-").replace('/', "_")).is_ok());
        assert!(parse_key("short").is_err());
        assert!(parse_key(&"00".repeat(16)).is_err());
        assert_eq!(format!("{:?}", Redacted("x".into())), "\"<redacted>\"");
    }

    fn app_db() -> Connection {
        freelib_catalog::open_app_db(Path::new(":memory:")).unwrap()
    }

    fn stored_pw(c: &Connection) -> Option<String> {
        let raw = db::get_setting_raw(c, "smtp").unwrap()?;
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        v["password"].as_str().map(String::from)
    }

    #[test]
    fn migrate_plain_then_idempotent_then_rotate() {
        let (a, b) = keys();
        let c = app_db();
        db::put_setting_raw(
            &c,
            "smtp",
            r#"{"host":"smtp.example.org","password":"plain-pw","port":587}"#,
        )
        .unwrap();
        let s = Secrets::from_keys(&a, None);
        let r = migrate(&c, &s).unwrap();
        assert_eq!(r.encrypted, 1);
        let e = stored_pw(&c).unwrap();
        assert!(e.starts_with("enc:v1:"), "{e}");
        let raw = db::get_setting_raw(&c, "smtp").unwrap().unwrap();
        assert!(!raw.contains("plain-pw"), "{raw}");
        assert!(raw.contains("smtp.example.org"), "other fields kept: {raw}");
        // idempotent
        let r = migrate(&c, &s).unwrap();
        assert_eq!(
            r,
            MigrateReport {
                current: 1,
                ..Default::default()
            }
        );
        assert_eq!(stored_pw(&c).unwrap(), e);
        // wrong key: refuse, and leave the value alone
        let wrong = Secrets::from_keys(&b, None);
        let err = migrate(&c, &wrong).unwrap_err().to_string();
        assert!(err.contains("FREELIB_SECRET_KEY_OLD"), "{err}");
        assert!(!err.contains("plain-pw"));
        assert_eq!(stored_pw(&c).unwrap(), e);
        // rotation
        let rot = Secrets::from_keys(&b, Some(&a));
        assert_eq!(migrate(&c, &rot).unwrap().rotated, 1);
        let e2 = stored_pw(&c).unwrap();
        assert_eq!(Secrets::key_id_of(&e2), Some(wrong.key_id()));
        assert_eq!(
            wrong.decrypt("smtp.password", &e2).unwrap().as_str(),
            "plain-pw"
        );
        // forget
        assert_eq!(forget(&c).unwrap(), 1);
        assert_eq!(stored_pw(&c), None);
        assert_eq!(migrate(&c, &wrong).unwrap(), MigrateReport::default());
    }

    #[test]
    fn key_file_generated_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let cfg = Config::for_dir(d.path());
        std::fs::create_dir_all(&cfg.data_dir).unwrap();
        let s1 = Secrets::load(&cfg).unwrap();
        let p = cfg.data_dir.join(KEY_FILE);
        assert!(matches!(s1.source, KeySource::Generated(_)));
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let s2 = Secrets::load(&cfg).unwrap();
        assert_eq!(s1.key_id(), s2.key_id(), "reused on the next start");
        // a loose mode is tightened
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        Secrets::load(&cfg).unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // env var wins and is not written anywhere
        let mut cfg2 = cfg.clone();
        cfg2.secret_key = Some(Redacted("11".repeat(32)));
        let s3 = Secrets::load(&cfg2).unwrap();
        assert_ne!(s3.key_id(), s1.key_id());
        assert_eq!(s3.source, KeySource::Env);
        // same old and new key is refused
        cfg2.secret_key_old = Some(Redacted("11".repeat(32)));
        assert!(Secrets::load(&cfg2).is_err());
        cfg2.secret_key = Some(Redacted("bad".into()));
        assert!(Secrets::load(&cfg2).is_err());
    }
}
