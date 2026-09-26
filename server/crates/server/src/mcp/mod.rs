//! MCP (Model Context Protocol) server at `/mcp`: the official Rust SDK (`rmcp`) with its
//! streamable HTTP transport in stateless mode (every POST is answered on its own, JSON
//! responses; protocol versions up to 2026-07-28).
//!
//! * **Auth**: `Authorization: Bearer fl_…` personal API tokens ([`crate::tokens`]) or OAuth
//!   access tokens `flo_…` ([`crate::oauth`]); the [`gate`] middleware checks the token, the
//!   admin switch `mcp.enabled` and the per-token rate limit, then hands the token to the
//!   handler through the request extensions. Without a valid token it answers 401 with
//!   `WWW-Authenticate: Bearer resource_metadata="…"` (MCP authorization discovery); an OAuth
//!   token calling a tool outside its scopes gets 403 `insufficient_scope` (step-up).
//! * **Tools** ([`tools`]): each declares a scope (`read`, `write`, `send`); `tools/list` shows
//!   the tools the token may call and every call is checked again and written to the audit log
//!   (Settings → Account → API tokens shows the last 50).
//! * **Prompts**: `suggest_next_book`, `books_for_kid`, `similar_to`.

pub mod suggest;
pub mod tools;

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rmcp::ErrorData as McpError;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult, ListToolsResult,
    PaginatedRequestParams, Prompt, PromptArgument, PromptMessage, Role, ServerCapabilities,
    ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Value, json};

use crate::db::{self, McpConfig};
use crate::state::AppState;
use crate::tokens::{self, TokenAuth};

/// Text sent to clients in `initialize` (`instructions`).
pub const INSTRUCTIONS: &str = "freeLib is a personal e-book library server (typically a \
Flibusta/Librusec-style collection of FB2/EPUB books, many in Russian). You act for one user.\n\
\n\
Ids: books, authors and series have numeric ids that are only valid inside one library; every \
catalog tool takes an optional `library` (default: the user's default library, see \
list_libraries) and returns the ids you need for the next call. Lists are paginated: pass the \
returned `nextCursor` as `cursor`.\n\
\n\
Ratings: `myRating` is the user's own rating (0 = not rated, 1..5); `libraryRating` is the \
rating that came with the library index (INPX, 0..5, 0 = none); `openLibrary` is the average \
and vote count on openlibrary.org (null when unknown; get_external_rating looks one up). \
`kidsAge` (\"0+\", \"6+\", \"12+\", \"16+\", \"18+\" or null) is a heuristic from genres and \
keywords, not a verified age rating.\n\
\n\
Good workflows: to suggest what to read next, call get_reading_profile, then \
suggest_candidates (it scores candidates server-side and explains why), inspect a few with \
get_book, and choose. To find books use search_books (text query and/or author, series, genre, \
language, date and rating filters). Books can be sent to the user's e-reader with send_books \
(e-mail devices like Kindle are limited by the server's allowed recipients and a daily limit); \
check the result with get_job. Ask the user before sending books or changing shelves or \
ratings unless they asked for it.";

/// Mounts `/mcp` (outside `/api/v1`: no cookies, bearer tokens only).
pub fn router(st: AppState) -> axum::Router<AppState> {
    let st2 = st.clone();
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(Some(std::time::Duration::from_secs(25)))
        // Host checks are done by our own `host_guard` (FREELIB_ALLOWED_HOSTS); the endpoint
        // needs a bearer token, so browsers' cookies and origins cannot be abused.
        .disable_allowed_hosts()
        .disable_allowed_origins()
        .with_max_request_body_bytes(crate::api::BODY_LIMIT);
    let service = StreamableHttpService::new(
        move || Ok(FreeLibMcp { st: st2.clone() }),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    axum::Router::new()
        .route_service("/mcp", service)
        .layer(axum::middleware::from_fn_with_state(st, gate))
}

fn json_error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({"error": code, "message": message})),
    )
        .into_response()
}

/// 401 with the `WWW-Authenticate` challenge: with OAuth available, the Protected Resource
/// Metadata URL and the scopes to ask for; `invalid_token` when a token was sent.
fn unauthorized(st: &AppState, message: &str, token_sent: bool) -> Response {
    let mut r = json_error(StatusCode::UNAUTHORIZED, "unauthorized", message);
    let mut v = String::from("Bearer realm=\"freeLib\"");
    if let Some(iss) = crate::oauth::issuer(st) {
        v.push_str(&format!(
            ", resource_metadata=\"{}\", scope=\"{}\"",
            crate::oauth::resource_metadata_url(&iss),
            tokens::SCOPES.join(" ")
        ));
    }
    if token_sent {
        v.push_str(
            ", error=\"invalid_token\", error_description=\"invalid, revoked or expired token\"",
        );
    }
    if let Ok(h) = header::HeaderValue::from_str(&v) {
        r.headers_mut().insert(header::WWW_AUTHENTICATE, h);
    }
    r
}

/// The scopes the tools called in a JSON-RPC body need (a message or a batch).
fn needed_scopes(body: &[u8]) -> Vec<&'static str> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    let msgs = match &v {
        Value::Array(a) => a.iter().collect(),
        m => vec![m],
    };
    let mut out = Vec::new();
    for m in msgs {
        if m.get("method").and_then(Value::as_str) == Some("tools/call")
            && let Some(s) = m
                .pointer("/params/name")
                .and_then(Value::as_str)
                .and_then(tools::scope_of)
            && !out.contains(&s)
        {
            out.push(s);
        }
    }
    out
}

/// Checks `mcp.enabled`, the bearer token and its rate limit; stores the [`TokenAuth`] in the
/// request extensions for the handler.
pub async fn gate(State(st): State<AppState>, mut req: Request<Body>, next: Next) -> Response {
    let enabled = st
        .db
        .run(|c| db::get_setting::<McpConfig>(c, "mcp"))
        .await
        .map(|m| m.enabled)
        .unwrap_or(false);
    if !enabled {
        return json_error(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the MCP endpoint is disabled by the administrator",
        );
    }
    let Some(secret) = tokens::bearer(req.headers()).map(str::to_string) else {
        return unauthorized(
            &st,
            "sign in (OAuth) or send an API token: Authorization: Bearer fl_…",
            false,
        );
    };
    let auth = match tokens::authenticate(&st, &secret).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return unauthorized(&st, "invalid, revoked or expired token", true);
        }
        Err(e) => return e.into_response(),
    };
    if let Err(wait) = st
        .tokens
        .check_rate(auth.rate_key(), st.cfg.mcp_rate_per_min)
    {
        let mut r = json_error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            &format!("too many requests with this token, retry in {wait} s"),
        );
        if let Ok(v) = header::HeaderValue::from_str(&wait.to_string()) {
            r.headers_mut().insert(header::RETRY_AFTER, v);
        }
        return r;
    }
    // OAuth clients get a step-up challenge for a tool outside the granted scopes
    if auth.is_oauth() && req.method() == axum::http::Method::POST {
        let (parts, body) = req.into_parts();
        let bytes = match axum::body::to_bytes(body, crate::api::BODY_LIMIT).await {
            Ok(b) => b,
            Err(_) => {
                return json_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "bad_request",
                    "request body too large",
                );
            }
        };
        let missing: Vec<&str> = needed_scopes(&bytes)
            .into_iter()
            .filter(|s| !auth.has(s))
            .collect();
        if !missing.is_empty() {
            return insufficient_scope(&st, &auth, &missing);
        }
        req = Request::from_parts(parts, Body::from(bytes));
    }
    req.extensions_mut().insert(auth);
    // HTTP/2 carries the host in the URI authority; the transport wants a Host header
    if !req.headers().contains_key(header::HOST)
        && let Some(v) = req
            .uri()
            .authority()
            .and_then(|a| header::HeaderValue::from_str(a.as_str()).ok())
    {
        req.headers_mut().insert(header::HOST, v);
    }
    next.run(req).await
}

/// 403 `insufficient_scope` naming every scope the client needs (granted + missing).
fn insufficient_scope(st: &AppState, auth: &TokenAuth, missing: &[&str]) -> Response {
    let all: Vec<&str> = tokens::SCOPES
        .iter()
        .copied()
        .filter(|s| auth.has(s) || missing.contains(s))
        .collect();
    let mut r = json_error(
        StatusCode::FORBIDDEN,
        "insufficient_scope",
        &format!("this app was not allowed '{}'", missing.join(" ")),
    );
    let mut v = format!(
        "Bearer realm=\"freeLib\", error=\"insufficient_scope\", scope=\"{}\"",
        all.join(" ")
    );
    if let Some(iss) = crate::oauth::issuer(st) {
        v.push_str(&format!(
            ", resource_metadata=\"{}\"",
            crate::oauth::resource_metadata_url(&iss)
        ));
    }
    if let Ok(h) = header::HeaderValue::from_str(&v) {
        r.headers_mut().insert(header::WWW_AUTHENTICATE, h);
    }
    r
}

/// The MCP handler (one per request in stateless mode).
#[derive(Clone)]
pub struct FreeLibMcp {
    pub st: AppState,
}

fn auth_of(ctx: &RequestContext<RoleServer>) -> Result<TokenAuth, McpError> {
    ctx.extensions
        .get::<axum::http::request::Parts>()
        .and_then(|p| p.extensions.get::<TokenAuth>())
        .cloned()
        .ok_or_else(|| McpError::invalid_request("not authenticated", None))
}

fn prompts() -> Vec<Prompt> {
    vec![
        Prompt::new(
            "suggest_next_book",
            Some(
                "Suggest what I should read next, from my library, based on my shelves, ratings and history.",
            ),
            Some(vec![
                PromptArgument::new("wishes")
                    .with_description(
                        "Optional: mood, genre or length you want, e.g. \"something light\"",
                    )
                    .with_required(false),
            ]),
        ),
        Prompt::new(
            "books_for_kid",
            Some("Find good books in the library for a child of a given age."),
            Some(vec![
                PromptArgument::new("age")
                    .with_description("The child's age in years")
                    .with_required(true),
                PromptArgument::new("interests")
                    .with_description("Optional interests, e.g. \"dinosaurs, space\"")
                    .with_required(false),
            ]),
        ),
        Prompt::new(
            "similar_to",
            Some("Find books similar to one book of the library."),
            Some(vec![
                PromptArgument::new("book_id")
                    .with_description("Book id (from search_books or get_book)")
                    .with_required(true),
            ]),
        ),
    ]
}

fn prompt_text(name: &str, args: &serde_json::Map<String, Value>) -> Result<String, McpError> {
    let arg = |k: &str| {
        args.get(k)
            .map(|v| match v {
                Value::String(s) => s.trim().to_string(),
                v => v.to_string(),
            })
            .filter(|s| !s.is_empty())
    };
    Ok(match name {
        "suggest_next_book" => {
            let wishes = arg("wishes")
                .map(|w| format!(" Take my wishes into account: {w}."))
                .unwrap_or_default();
            format!(
                "Suggest what I should read next from my freeLib library.{wishes}\n\n\
                 1. Call get_reading_profile to see my shelves, ratings and what I sent or read recently.\n\
                 2. Call suggest_candidates with use_profile=true (exclude what I already read; add genre or \
                 language constraints if my wishes imply them).\n\
                 3. Look at the best 5–8 candidates with get_book (annotation, ratings) and pick the 3 best \
                 for me. Prefer continuing series I read, then authors I like, then well-rated books in my genres.\n\
                 4. Answer with the 3 books (title, author, series and number, one or two sentences why, \
                 ratings) and offer to send one to my e-reader with send_books. Do not send without asking."
            )
        }
        "books_for_kid" => {
            let age =
                arg("age").ok_or_else(|| McpError::invalid_params("age is required", None))?;
            let age_n: u8 = age
                .trim_end_matches('+')
                .parse()
                .map_err(|_| McpError::invalid_params("age must be a number", None))?;
            let interests = arg("interests")
                .map(|i| format!(" The child likes: {i}."))
                .unwrap_or_default();
            format!(
                "Find books in my freeLib library for a {age_n}-year-old child.{interests}\n\n\
                 Use search_books and suggest_candidates with kids_max_age={age_n} (only books whose \
                 heuristic age estimate is known and at most {age_n}); prefer books with a library or \
                 Open Library rating, and the child's language (ask if unsure). Check annotations with \
                 get_book: the age estimate comes from genres and keywords and can be wrong, so skip \
                 anything that looks unsuitable. Propose 5 books with a short reason each and offer to \
                 put them on a shelf (add_to_shelf) or send one (send_books)."
            )
        }
        "similar_to" => {
            let id = arg("book_id")
                .ok_or_else(|| McpError::invalid_params("book_id is required", None))?;
            format!(
                "Find books in my freeLib library similar to book {id}.\n\n\
                 1. Call get_book with id {id} to learn its authors, series, genres and annotation.\n\
                 2. Call suggest_candidates with seed_book_ids=[{id}] and use_profile=false; also try \
                 search_books with the book's main genre and a minimum rating.\n\
                 3. Compare annotations (get_book) and return the 5 most similar books with one sentence \
                 each on what they share, skipping books I already read."
            )
        }
        _ => {
            return Err(McpError::invalid_params(
                format!("unknown prompt {name}"),
                None,
            ));
        }
    })
}

impl ServerHandler for FreeLibMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_server_info(
            Implementation::new("freelib", env!("CARGO_PKG_VERSION")).with_title("freeLib library"),
        )
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let auth = auth_of(&ctx)?;
        Ok(ListToolsResult::with_all_items(
            tools::definitions()
                .into_iter()
                .filter(|(_, scope)| auth.has(scope))
                .map(|(t, _)| t)
                .collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let auth = auth_of(&ctx)?;
        let name = request.name.to_string();
        let args = Value::Object(request.arguments.unwrap_or_default());
        let detail: String = args.to_string().chars().take(200).collect();
        let result = match tools::scope_of(&name) {
            None => Err(format!("unknown tool {name}")),
            Some(scope) if !auth.has(scope) => Err(format!(
                "this API token lacks the '{scope}' scope needed by {name}; create a token with it in Settings > Account"
            )),
            Some(_) => tools::call(&self.st, &auth, &name, args).await,
        };
        let ok = result.is_ok();
        let (uid, tid, gid, tool) = (auth.user.id, auth.token_id(), auth.grant_id(), name.clone());
        let _ = self
            .st
            .db
            .run(move |c| db::add_audit(c, uid, tid, gid, &tool, ok, &detail))
            .await;
        Ok(match result {
            Ok(v) => CallToolResult::structured(v).into(),
            Err(msg) => CallToolResult::error(vec![ContentBlock::text(msg)]).into(),
        })
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        auth_of(&ctx)?;
        Ok(ListPromptsResult::with_all_items(prompts()))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        auth_of(&ctx)?;
        let args = request.arguments.clone().unwrap_or_default();
        let text = prompt_text(&request.name, &args)?;
        let desc = prompts()
            .into_iter()
            .find(|p| p.name == request.name)
            .and_then(|p| p.description);
        let mut r = GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)]);
        if let Some(d) = desc {
            r = r.with_description(d);
        }
        Ok(r.into())
    }
}
