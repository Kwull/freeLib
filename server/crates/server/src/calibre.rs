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

    /// `ebook-convert input output [args]` inside `work` (absolute paths we created), with a
    /// timeout; the output extension selects the format. `args` are `--option=value` pairs (see
    /// [`metadata_args`]). Setting `cancel` kills the process.
    pub async fn convert(
        &self,
        input: &Path,
        output: &Path,
        work: &Path,
        args: &[String],
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
            .args(args)
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

/// One `--name=value` option; values are single-line (control characters dropped). The `=`
/// form keeps a value that starts with `-` from being read as another option.
fn opt(name: &str, value: &str) -> Option<String> {
    let v: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!v.is_empty()).then(|| format!("--{name}={v}"))
}

/// Calibre metadata options for a book (read back from our own EPUB): title and title sort,
/// authors (`&`-separated, as Calibre expects) and author sort, series and index, language,
/// publisher, ISBN, publication date. Kindle shows these from the AZW3's EXTH header.
pub fn metadata_args(info: &freelib_fb2conv::BookInfo) -> Vec<String> {
    let lang = freelib_fb2conv::normalize_language(&info.lang).unwrap_or_default();
    let names = |f: &dyn Fn(&freelib_fb2conv::Person) -> String| {
        info.authors
            .iter()
            .map(|a| f(a).replace('&', "and"))
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" & ")
    };
    let mut v = Vec::new();
    v.extend(opt("title", &info.title));
    v.extend(opt(
        "title-sort",
        &freelib_fb2conv::title_sort(&info.title, &lang),
    ));
    v.extend(opt("authors", &names(&|a| a.natural_name())));
    v.extend(opt("author-sort", &names(&|a| a.file_as())));
    if let Some(s) = &info.series {
        v.extend(opt("series", s));
        if let Some(n) = info.serno {
            v.extend(opt("series-index", &n.to_string()));
        }
    }
    v.extend(opt("language", &lang));
    v.extend(opt("publisher", &info.publisher));
    v.extend(opt("isbn", &info.isbn.replace([' ', '-'], "")));
    let date = [&info.date, &info.year]
        .into_iter()
        .map(|d| d.trim())
        .find(|d| d.len() >= 4 && d[..4].chars().all(|c| c.is_ascii_digit()))
        .map(|d| {
            if d.len() == 10 {
                d.to_string()
            } else {
                d[..4].to_string()
            }
        });
    if let Some(d) = date {
        v.extend(opt("pubdate", &d));
    }
    v.extend(opt("book-producer", "freeLib"));
    v
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

    #[test]
    fn metadata_arguments() {
        use freelib_fb2conv::{BookInfo, Person};
        let info = BookInfo {
            title: "The Hobbit".into(),
            authors: vec![
                Person {
                    first: "John".into(),
                    middle: "Ronald Reuel".into(),
                    last: "Tolkien".into(),
                    ..Default::default()
                },
                Person {
                    first: "Ann".into(),
                    last: "Smith & Co".into(),
                    ..Default::default()
                },
            ],
            series: Some("Middle-earth".into()),
            serno: Some(1),
            lang: "eng".into(),
            isbn: "978-0-261-10221-7".into(),
            date: "1937".into(),
            publisher: "--Allen\n& Unwin".into(),
            ..Default::default()
        };
        assert_eq!(
            metadata_args(&info),
            [
                "--title=The Hobbit",
                "--title-sort=Hobbit, The",
                "--authors=John Ronald Reuel Tolkien & Ann Smith and Co",
                "--author-sort=Tolkien, John Ronald Reuel & Smith and Co, Ann",
                "--series=Middle-earth",
                "--series-index=1",
                "--language=en",
                "--publisher=--Allen & Unwin",
                "--isbn=9780261102217",
                "--pubdate=1937",
                "--book-producer=freeLib",
            ]
        );
        // nothing known: only the producer
        assert_eq!(
            metadata_args(&BookInfo::default()),
            ["--book-producer=freeLib"]
        );
    }

    /// A fake `ebook-convert` that records its arguments.
    #[tokio::test]
    async fn arguments_reach_calibre() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ebook-convert");
        std::fs::write(
            &p,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'ebook-convert (calibre 8.5.0)'; exit 0; fi\n\
             for a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"$(dirname \"$2\")/args.txt\"\ncp \"$1\" \"$2\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let c = Calibre::detect(&p).await.unwrap();
        let (inp, out) = (dir.path().join("in.epub"), dir.path().join("out.azw3"));
        std::fs::write(&inp, b"epub").unwrap();
        let args = vec!["--title=-x; rm -rf /".to_string(), "--series=S".to_string()];
        c.convert(&inp, &out, dir.path(), &args, Duration::from_secs(20), None)
            .await
            .unwrap();
        let got = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        let lines: Vec<&str> = got.lines().collect();
        assert_eq!(lines[2..], ["--title=-x; rm -rf /", "--series=S"]);
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
