//! Calibre `ebook-convert` subprocess (EPUB → AZW3 / MOBI / PDF).
//!
//! Hardening: versions before 6.19 (CVE-2023-46303, arbitrary file write via HTML input) are
//! refused at startup; the server only ever feeds it EPUB files it wrote itself; every run gets
//! its own temporary directory as cwd, `HOME` and Calibre config/temp directory; a cancelled job
//! kills the process.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::ApiError;

/// Oldest Calibre accepted (fixes CVE-2023-46303).
pub const MIN_VERSION: (u32, u32) = (6, 19);

#[derive(Debug, Clone)]
pub struct Calibre {
    pub path: PathBuf,
    pub version: Option<String>,
}

impl Calibre {
    /// Runs `ebook-convert --version`; `None` when it cannot be executed or is too old.
    pub async fn detect(path: &Path) -> Option<Calibre> {
        let out = tokio::time::timeout(
            Duration::from_secs(30),
            Command::new(path)
                .arg("--version")
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        if !out.status.success() {
            tracing::warn!(
                "{} --version failed; Calibre conversions disabled",
                path.display()
            );
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let version = parse_version(&text);
        match version.as_deref().and_then(numeric_version) {
            Some(v) if v >= MIN_VERSION => {}
            _ => {
                tracing::warn!(
                    "Calibre {} at {} is older than {}.{} (or its version is unknown) and has known \
                     vulnerabilities (CVE-2023-46303): AZW3/MOBI/PDF disabled; install a newer Calibre",
                    version.as_deref().unwrap_or("?"),
                    path.display(),
                    MIN_VERSION.0,
                    MIN_VERSION.1
                );
                return None;
            }
        }
        Some(Calibre {
            path: path.to_path_buf(),
            version,
        })
    }

    /// `ebook-convert input output` inside `work` (absolute paths we created), with a timeout;
    /// the output extension selects the format. Setting `cancel` kills the process.
    pub async fn convert(
        &self,
        input: &Path,
        output: &Path,
        work: &Path,
        timeout: Duration,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<(), ApiError> {
        if !input.is_absolute() || !output.is_absolute() {
            return Err(ApiError::internal("Calibre needs absolute paths"));
        }
        let home = work.join("home");
        let config = home.join("config");
        let temp = work.join("temp");
        for d in [&config, &temp] {
            tokio::fs::create_dir_all(d).await?;
        }
        let mut child = Command::new(&self.path)
            .arg(input)
            .arg(output)
            .current_dir(work)
            .env("HOME", &home)
            .env("CALIBRE_CONFIG_DIRECTORY", &config)
            .env("CALIBRE_TEMP_DIR", &temp)
            .env("TMPDIR", &temp)
            .env(
                "QT_QPA_PLATFORM",
                std::env::var("QT_QPA_PLATFORM").unwrap_or_else(|_| "offscreen".into()),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ApiError::internal(format!("cannot start Calibre: {e}")))?;
        let mut stderr = child.stderr.take();
        let err_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(s) = stderr.as_mut() {
                // keep only a bounded amount of diagnostics
                let _ = s.take(256 * 1024).read_to_end(&mut buf).await;
            }
            buf
        });
        let deadline = Instant::now() + timeout;
        let status = loop {
            let cancelled = cancel.is_some_and(|c| c.load(Ordering::SeqCst));
            if cancelled || Instant::now() >= deadline {
                let _ = child.kill().await;
                err_task.abort();
                return Err(if cancelled {
                    ApiError::new(axum::http::StatusCode::OK, "cancelled", "cancelled")
                } else {
                    ApiError::internal("Calibre conversion timed out")
                });
            }
            match tokio::time::timeout(Duration::from_millis(250), child.wait()).await {
                Ok(r) => break r.map_err(|e| ApiError::internal(format!("Calibre: {e}")))?,
                Err(_) => continue,
            }
        };
        let err = err_task.await.unwrap_or_default();
        if !status.success() || !output.is_file() {
            let err = String::from_utf8_lossy(&err);
            let tail: String = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
            return Err(ApiError::internal(format!(
                "Calibre conversion failed: {tail}"
            )));
        }
        Ok(())
    }
}

/// `ebook-convert (calibre 7.4.0)` → `7.4.0`.
fn parse_version(s: &str) -> Option<String> {
    let line = s.lines().next()?.trim();
    if let Some(i) = line.find("calibre ") {
        let v: String = line[i + 8..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if !v.is_empty() {
            return Some(v);
        }
    }
    (!line.is_empty()).then(|| line.to_string())
}

/// `7.4.0` → `(7, 4)`.
fn numeric_version(v: &str) -> Option<(u32, u32)> {
    let mut it = v.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next().map_or(Some(0), |m| m.parse().ok())?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version() {
        assert_eq!(
            parse_version("ebook-convert (calibre 7.4.0)\nCreated by: Kovid Goyal").unwrap(),
            "7.4.0"
        );
        assert_eq!(numeric_version("6.13.0"), Some((6, 13)));
        assert!(numeric_version("6.13.0").unwrap() < MIN_VERSION);
        assert!(numeric_version("6.19").unwrap() >= MIN_VERSION);
        assert!(numeric_version("8.5.0").unwrap() >= MIN_VERSION);
        assert_eq!(numeric_version("7"), Some((7, 0)));
        assert_eq!(numeric_version("garbage"), None);
    }

    #[tokio::test]
    async fn old_calibre_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ebook-convert");
        std::fs::write(&p, "#!/bin/sh\necho 'ebook-convert (calibre 6.13.0)'\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Calibre::detect(&p).await.is_none());
        std::fs::write(&p, "#!/bin/sh\necho 'ebook-convert (calibre 8.5.0)'\n").unwrap();
        assert_eq!(
            Calibre::detect(&p).await.unwrap().version.as_deref(),
            Some("8.5.0")
        );
    }
}
