//! SMTP (Send to Kindle) via lettre with rustls.

use std::time::Duration;

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::db::SmtpConfig;
use crate::error::{ApiError, ApiResult};

fn transport(cfg: &SmtpConfig) -> ApiResult<AsyncSmtpTransport<Tokio1Executor>> {
    if cfg.host.trim().is_empty() {
        return Err(ApiError::bad_request("SMTP server is not configured"));
    }
    let host = cfg.host.trim();
    let tls = || {
        TlsParameters::new(host.to_string()).map_err(|e| ApiError::bad_request(format!("TLS: {e}")))
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
    Ok(b.timeout(Some(Duration::from_secs(60))).build())
}

fn mailbox(s: &str, what: &str) -> ApiResult<Mailbox> {
    s.trim()
        .parse()
        .map_err(|_| ApiError::bad_request(format!("invalid {what} address: {s}")))
}

/// Sends one mail; `attachment` = (file name, MIME type, bytes).
pub async fn send(
    cfg: &SmtpConfig,
    to: &str,
    subject: &str,
    text: &str,
    attachment: Option<(&str, &str, Vec<u8>)>,
) -> ApiResult<()> {
    let from = if cfg.from.trim().is_empty() {
        cfg.username.as_str()
    } else {
        cfg.from.as_str()
    };
    let builder = Message::builder()
        .from(mailbox(from, "sender")?)
        .to(mailbox(to, "recipient")?)
        .subject(subject);
    let body = SinglePart::plain(text.to_string());
    let msg = match attachment {
        Some((name, mime, data)) => {
            let ct = ContentType::parse(mime)
                .unwrap_or(ContentType::parse("application/octet-stream").expect("valid"));
            builder.multipart(
                MultiPart::mixed()
                    .singlepart(body)
                    .singlepart(Attachment::new(name.to_string()).body(data, ct)),
            )
        }
        None => builder.singlepart(body),
    }
    .map_err(|e| ApiError::bad_request(format!("mail: {e}")))?;
    transport(cfg)?
        .send(msg)
        .await
        .map_err(|e| ApiError::bad_request(format!("SMTP: {e}")))?;
    Ok(())
}
