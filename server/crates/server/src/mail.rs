//! SMTP (Send to Kindle) via lettre with rustls.
//!
//! [`deliver`] reports what the mail server said: the accepting reply (e.g.
//! `250 2.0.0 Ok: queued as 4F3…`) or a [`MailError`] classified as temporary (4xx replies,
//! timeouts, refused or dropped connections: worth a retry) or permanent (5xx replies such as
//! a failed login or a rejected recipient, TLS and message errors: retrying cannot help).

use std::time::Duration;

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::db::SmtpConfig;
use crate::error::{ApiError, ApiResult};

/// A failed delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailError {
    /// Retrying cannot help (5xx, TLS, invalid message or configuration).
    pub permanent: bool,
    pub message: String,
}

impl MailError {
    fn permanent(message: impl Into<String>) -> MailError {
        MailError {
            permanent: true,
            message: message.into(),
        }
    }
}

impl From<MailError> for ApiError {
    fn from(e: MailError) -> ApiError {
        ApiError::bad_request(e.message)
    }
}

/// Whether an SMTP error is permanent (see the module docs).
pub fn is_permanent(e: &lettre::transport::smtp::Error) -> bool {
    e.is_permanent() || e.is_client() || e.is_tls() || e.is_response()
}

fn transport(cfg: &SmtpConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>, MailError> {
    if cfg.host.trim().is_empty() {
        return Err(MailError::permanent("SMTP server is not configured"));
    }
    let host = cfg.host.trim();
    let tls = || {
        TlsParameters::new(host.to_string()).map_err(|e| MailError::permanent(format!("TLS: {e}")))
    };
    let mut b = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host).port(cfg.port);
    b = match cfg.security.as_str() {
        "tls" => b.tls(Tls::Wrapper(tls()?)),
        "starttls" => b.tls(Tls::Required(tls()?)),
        _ => b.tls(Tls::None),
    };
    if !cfg.username.is_empty() {
        b = b.credentials(Credentials::new(
            cfg.username.clone(),
            cfg.password.clone().unwrap_or_default(),
        ));
    }
    // large attachments over slow uplinks need time
    Ok(b.timeout(Some(Duration::from_secs(300))).build())
}

fn mailbox(s: &str, what: &str) -> Result<Mailbox, MailError> {
    s.trim()
        .parse()
        .map_err(|_| MailError::permanent(format!("invalid {what} address: {s}")))
}

/// The sender address (`from`, or the SMTP user name).
pub fn from_address(cfg: &SmtpConfig) -> &str {
    if cfg.from.trim().is_empty() {
        cfg.username.trim()
    } else {
        cfg.from.trim()
    }
}

/// One attachment: file name, MIME type, bytes.
pub type Attach = (String, String, Vec<u8>);

/// Sends one mail with any number of attachments; returns the server's accepting reply.
pub async fn deliver(
    cfg: &SmtpConfig,
    to: &str,
    subject: &str,
    text: &str,
    attachments: Vec<Attach>,
) -> Result<String, MailError> {
    let builder = Message::builder()
        .from(mailbox(from_address(cfg), "sender")?)
        .to(mailbox(to, "recipient")?)
        .subject(subject);
    let body = SinglePart::plain(text.to_string());
    let msg = if attachments.is_empty() {
        builder.singlepart(body)
    } else {
        let mut mp = MultiPart::mixed().singlepart(body);
        for (name, mime, data) in attachments {
            let ct = ContentType::parse(&mime)
                .unwrap_or(ContentType::parse("application/octet-stream").expect("valid"));
            mp = mp.singlepart(Attachment::new(name).body(data, ct));
        }
        builder.multipart(mp)
    }
    .map_err(|e| MailError::permanent(format!("mail: {e}")))?;
    match transport(cfg)?.send(msg).await {
        Ok(r) => {
            let lines: Vec<&str> = r.message().collect();
            Ok(format!("{} {}", r.code(), lines.join(" "))
                .trim()
                .to_string())
        }
        Err(e) => Err(MailError {
            permanent: is_permanent(&e),
            message: format!("SMTP: {e}"),
        }),
    }
}

/// Sends one mail; `attachment` = (file name, MIME type, bytes). Errors become 400.
pub async fn send(
    cfg: &SmtpConfig,
    to: &str,
    subject: &str,
    text: &str,
    attachment: Option<(&str, &str, Vec<u8>)>,
) -> ApiResult<()> {
    let att = attachment
        .map(|(n, m, d)| vec![(n.to_string(), m.to_string(), d)])
        .unwrap_or_default();
    deliver(cfg, to, subject, text, att).await?;
    Ok(())
}
