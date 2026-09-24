//! Calibre `ebook-convert` subprocess (AZW3 / MOBI / PDF and non-FB2 inputs).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::error::ApiError;

#[derive(Debug, Clone)]
pub struct Calibre {
    pub path: PathBuf,
    pub version: Option<String>,
}

impl Calibre {
    /// Runs `ebook-convert --version`; `None` when it cannot be executed.
    pub async fn detect(path: &Path) -> Option<Calibre> {
        let out = tokio::time::timeout(
            Duration::from_secs(30),
            Command::new(path).arg("--version").stdin(Stdio::null()).kill_on_drop(true).output(),
        )
        .await
        .ok()?
        .ok()?;
        if !out.status.success() {
            tracing::warn!("{} --version failed; Calibre conversions disabled", path.display());
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Some(Calibre { path: path.to_path_buf(), version: parse_version(&text) })
    }

    /// `ebook-convert input output` with a timeout; the output extension selects the format.
    pub async fn convert(&self, input: &Path, output: &Path, timeout: Duration) -> Result<(), ApiError> {
        let child = Command::new(&self.path)
            .arg(input)
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("QT_QPA_PLATFORM", std::env::var("QT_QPA_PLATFORM").unwrap_or_else(|_| "offscreen".into()))
            .spawn()
            .map_err(|e| ApiError::internal(format!("cannot start Calibre: {e}")))?;
        let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(r) => r.map_err(|e| ApiError::internal(format!("Calibre: {e}")))?,
            Err(_) => return Err(ApiError::internal("Calibre conversion timed out")),
        };
        if !out.status.success() || !output.is_file() {
            let err = String::from_utf8_lossy(&out.stderr);
            let tail: String = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
            return Err(ApiError::internal(format!("Calibre conversion failed: {tail}")));
        }
        Ok(())
    }
}

/// `ebook-convert (calibre 7.4.0)` → `7.4.0`.
fn parse_version(s: &str) -> Option<String> {
    let line = s.lines().next()?.trim();
    if let Some(i) = line.find("calibre ") {
        let v: String = line[i + 8..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        if !v.is_empty() {
            return Some(v);
        }
    }
    (!line.is_empty()).then(|| line.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn version() {
        assert_eq!(super::parse_version("ebook-convert (calibre 7.4.0)\nCreated by: Kovid Goyal").unwrap(), "7.4.0");
    }
}
