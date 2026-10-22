//! HTTP: routing, handlers and the small amount of per-request state.

pub mod settings;
pub mod views;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts, Path, Query, Request, State};
use axum::http::header::{self, HeaderValue};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;

use crate::db::Store;
use tokio::sync::Semaphore;
use tokio_stream::wrappers::{BroadcastStream, IntervalStream};
use tokio_stream::StreamExt;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::auth::{self, CurrentUser};
use crate::config::Config;
use crate::error::AppError;
use crate::events::{EventKind, Events};
use crate::go::{sgf, Cell, Point};
use crate::i18n::{self, Lang};
use crate::store::{self, StoreError};
use crate::util::{date_ymd, now_unix};

#[derive(Clone)]
pub struct AppState {
    pub db: Store,
    pub config: Arc<Config>,
    /// Argon2id is memory hard, so only a few hashes may run at once.
    pub hasher: Arc<Semaphore>,
    pub login_limiter: Arc<auth::RateLimiter>,
    pub register_limiter: Arc<auth::RateLimiter>,
    /// Live updates for open pages.
    pub events: Events,
    /// Set when the server is stopping, so event streams can end.
    pub shutdown: Arc<tokio::sync::watch::Sender<bool>>,
}

impl AppState {
    /// Ask every open event stream to end, so the server can stop promptly.
    pub fn shut_down(&self) {
        let _ = self.shutdown.send(true);
    }

    pub fn new(db: Store, config: Config) -> Self {
        let login_limiter = auth::RateLimiter::new(
            config.limits.login_attempts,
            config.limits.login_window_minutes * 60,
        );
        let register_limiter = auth::RateLimiter::new(
            config.limits.registration_attempts,
            config.limits.registration_window_minutes * 60,
        );
        AppState {
            db,
            config: Arc::new(config),
            hasher: Arc::new(Semaphore::new(4)),
            login_limiter: Arc::new(login_limiter),
            register_limiter: Arc::new(register_limiter),
            events: Events::new(),
            shutdown: Arc::new(tokio::sync::watch::channel(false).0),
        }
    }
}

pub fn router(state: AppState) -> Router {
    settings::init(&state.config);
    // The language layer needs the state, but `with_state` consumes it below.
    let language_state = state.clone();
    Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/static/app.css", get(stylesheet))
        .route("/static/app.js", get(script))
        .route("/games", post(create_game))
        .route("/games/{id}/claim", post(claim_seat))
        .route("/games/{id}/withdraw", post(withdraw_game))
        .route("/g/{id}", get(show_game))
        .route("/g/{id}/move", post(play_move))
        .route("/g/{id}/pass", post(play_pass))
        .route("/g/{id}/version", get(game_version))
        .route("/stream", get(stream))
        .route("/lang/{code}", get(set_language))
        .route("/g/{id}/sgf", get(game_sgf))
        .route("/register", get(register_form).post(register))
        .route("/login", get(login_form).post(login))
        .route("/logout", post(logout))
        .route("/me", get(account))
        .route("/me/password", post(change_password))
        .route("/me/logout-all", post(logout_everywhere))
        .fallback(not_found)
        .with_state(state)
        .layer(middleware::from_fn(guard_state_changing_requests))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(
            language_state,
            with_language,
        ))
}

/// Refuse cross-site POSTs. `SameSite=Lax` on the session cookie already blocks
/// most of this; this is the explicit half of the check.
async fn guard_state_changing_requests(request: Request, next: Next) -> Response {
    if !matches!(request.method(), &Method::GET | &Method::HEAD)
        && !auth::same_origin(request.headers())
    {
        return AppError::Forbidden(
            i18n::t(i18n::current(), crate::i18n::Text::CrossSite).to_owned(),
        )
        .into_response();
    }
    next.run(request).await
}

/// The client address used for rate limiting, honouring forwarding headers only
/// when the operator says the server sits behind a proxy.
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if state.config.trust_proxy {
            for name in ["x-forwarded-for", "x-real-ip"] {
                if let Some(value) = parts
                    .headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                {
                    if let Some(first) = value.split(',').next().map(str::trim) {
                        if !first.is_empty() {
                            return Ok(ClientIp(first.to_owned()));
                        }
                    }
                }
            }
        }
        let address = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip().to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        Ok(ClientIp(address))
    }
}

fn redirect_with_cookie(location: &str, cookie: String) -> Response {
    let mut response = Redirect::to(location).into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

/// Only allow relative, single-slash targets so that `?next=` cannot be used as
/// an open redirect.
fn next_target(next: &str) -> String {
    let trimmed = next.trim();
    if trimmed.starts_with('/') && !trimmed.starts_with("//") && !trimmed.contains('\\') {
        trimmed.to_owned()
    } else {
        "/".to_owned()
    }
}

// ------------------------------------------------------------- language

/// Cookie holding the visitor's language choice. A cookie rather than an
/// account field: visitors who only watch games can pick a language too.
const LANGUAGE_COOKIE: &str = "komi_lang";

/// How long that choice is remembered.
const LANGUAGE_COOKIE_MAX_AGE: i64 = 400 * 24 * 60 * 60;

/// The language for this request: the cookie, then the browser's preference,
/// then the configured default.
fn resolve_language(config: &Config, headers: &HeaderMap) -> Lang {
    if let Some(lang) =
        auth::read_cookie(headers, LANGUAGE_COOKIE).and_then(|value| Lang::parse(&value))
    {
        return lang;
    }
    headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .and_then(Lang::from_accept_language)
        .unwrap_or_else(|| Lang::parse(&config.default_language).unwrap_or_default())
}

/// Wraps every request in the language scope, so that the generic error page —
/// which is rendered outside a handler — can translate itself too.
async fn with_language(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let lang = resolve_language(&state.config, request.headers());
    i18n::scope(lang, next.run(request)).await
}

/// Switch language, then return to the page the visitor came from.
async fn set_language(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(lang) = Lang::parse(&code) else {
        return Err(AppError::NotFound);
    };
    let mut response = Redirect::to(&referer_path(&headers)).into_response();
    let mut cookie = format!(
        "{LANGUAGE_COOKIE}={}; Path=/; Max-Age={LANGUAGE_COOKIE_MAX_AGE}; SameSite=Lax",
        lang.code()
    );
    if state.config.secure_cookies {
        cookie.push_str("; Secure");
    }
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    Ok(response)
}

/// Where to send the browser back to: the referring path on any host, but only
/// if it is a plain single-slash path, so this cannot become an open redirect.
fn referer_path(headers: &HeaderMap) -> String {
    headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|referer| referer.split_once("://").map(|(_, rest)| rest))
        .and_then(|rest| rest.find('/').map(|index| rest[index..].to_owned()))
        .filter(|path| path.starts_with('/') && !path.starts_with("//") && !path.contains('\\'))
        .unwrap_or_else(|| "/".to_owned())
}

// ------------------------------------------------------------ static files

async fn healthz() -> &'static str {
    "ok"
}

async fn stylesheet() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        crate::assets::STYLESHEET,
    )
        .into_response()
}

async fn script() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        crate::assets::SCRIPT,
    )
        .into_response()
}

async fn not_found() -> AppError {
    AppError::NotFound
}

// ---------------------------------------------------------------- lobby

#[derive(Deserialize, Default)]
struct LobbyQuery {
    #[serde(default)]
    filter: String,
    #[serde(default)]
    sort: String,
}

async fn index(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<LobbyQuery>,
) -> Result<Html<String>, AppError> {
    let lang = i18n::current();
    let now = now_unix();
    let filter = store::GameFilter::parse(&query.filter, current.id());
    let sort = store::GameSort::parse(&query.sort);
    let games = store::games(
        &state.db,
        store::GameQuery {
            filter,
            sort,
            limit: 60,
            now,
            stale_after: state.config.stale_open_seconds(),
        },
    )
    .await?;
    Ok(Html(
        views::index_page(
            &current,
            &games,
            filter,
            sort,
            now,
            state.config.allow_registration,
            lang,
        )
        .into_string(),
    ))
}

#[derive(Deserialize)]
struct CreateGameForm {
    #[serde(default)]
    opponent: String,
    /// "black" or "white"; black is the default because it moves first.
    #[serde(default)]
    color: String,
}

async fn create_game(
    State(state): State<AppState>,
    current: CurrentUser,
    Form(form): Form<CreateGameForm>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let session = current.require()?;
    let colour = match form.color.trim().to_ascii_lowercase().as_str() {
        "" | "black" | "b" => Cell::Black,
        "white" | "w" => Cell::White,
        other => {
            return Err(AppError::BadRequest(i18n::fill(
                i18n::t(lang, crate::i18n::Text::NotAColour),
                &[("value", other)],
            )))
        }
    };
    let name = form.opponent.trim();
    let opponent = if name.is_empty() {
        None
    } else {
        let Some(record) = store::user_by_username(&state.db, name).await? else {
            return Err(AppError::BadRequest(i18n::fill(
                i18n::t(lang, crate::i18n::Text::NoSuchPlayer),
                &[("name", name)],
            )));
        };
        if record.user.id == session.user.id {
            return Err(AppError::BadRequest(
                i18n::t(lang, crate::i18n::Text::CannotInviteSelf).to_owned(),
            ));
        }
        Some(record.user)
    };
    let game_id = store::create_game(
        &state.db,
        session.user.id,
        colour,
        opponent.as_ref(),
        now_unix(),
    )
    .await?;
    state.events.publish(game_id, EventKind::Created);
    Ok(Redirect::to(&format!("/g/{game_id}")).into_response())
}

async fn claim_seat(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(game_id): Path<i64>,
) -> Result<Response, AppError> {
    let session = current.require()?;
    store::claim_seat(&state.db, game_id, session.user.id, now_unix()).await?;
    state.events.publish(game_id, EventKind::Claimed);
    Ok(Redirect::to(&format!("/g/{game_id}")).into_response())
}

// ----------------------------------------------------------------- game

async fn withdraw_game(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(game_id): Path<i64>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let session = current.require()?;
    match store::withdraw_game(&state.db, game_id, session.user.id).await {
        Ok(()) => {
            state.events.publish(game_id, EventKind::Withdrawn);
            Ok(Redirect::to("/").into_response())
        }
        Err(error) if is_rule_refusal(&error) => Ok((
            StatusCode::BAD_REQUEST,
            Html(
                views::game_error_page(
                    lang,
                    views::Nav::from(&current),
                    state.config.allow_registration,
                    &format!("/g/{game_id}"),
                    i18n::t(lang, crate::error::store_text(&error)),
                )
                .into_string(),
            ),
        )
            .into_response()),
        Err(error) => Err(error.into()),
    }
}

async fn show_game(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(game_id): Path<i64>,
) -> Result<Html<String>, AppError> {
    let lang = i18n::current();
    let game = store::game(&state.db, game_id).await?;
    let moves = store::moves(&state.db, game_id).await?;
    let board = game.position.clone().unwrap_or_default();
    Ok(Html(
        views::game_page(
            &game,
            &moves,
            &board,
            &current,
            now_unix(),
            state.config.allow_registration,
            lang,
        )
        .into_string(),
    ))
}

#[derive(Deserialize)]
struct MoveForm {
    seq: u32,
    point: String,
}

#[derive(Deserialize)]
struct SeqForm {
    seq: u32,
}

async fn play_move(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(game_id): Path<i64>,
    Form(form): Form<MoveForm>,
) -> Result<Response, AppError> {
    current.require()?;
    let lang = i18n::current();
    let point = Point::parse_label(&form.point).ok_or_else(|| {
        AppError::BadRequest(i18n::fill(
            i18n::t(lang, crate::i18n::Text::NotAPoint),
            &[("value", &form.point)],
        ))
    })?;
    play_and_redirect(&state, &current, game_id, form.seq, Some(point)).await
}

async fn play_pass(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(game_id): Path<i64>,
    Form(form): Form<SeqForm>,
) -> Result<Response, AppError> {
    current.require()?;
    play_and_redirect(&state, &current, game_id, form.seq, None).await
}

async fn play_and_redirect(
    state: &AppState,
    current: &CurrentUser,
    game_id: i64,
    seq: u32,
    point: Option<Point>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let session = current.require()?;
    match store::play(&state.db, game_id, session.user.id, seq, point, now_unix()).await {
        Ok(_) => {
            state.events.publish(game_id, EventKind::Played);
            Ok(Redirect::to(&format!("/g/{game_id}")).into_response())
        }
        // Ordinary rule refusals are not server errors: show them on a page that
        // leads straight back to the board.
        Err(error) if is_rule_refusal(&error) => Ok((
            StatusCode::BAD_REQUEST,
            Html(
                views::game_error_page(
                    lang,
                    views::Nav::from(current),
                    state.config.allow_registration,
                    &format!("/g/{game_id}"),
                    i18n::t(lang, crate::error::store_text(&error)),
                )
                .into_string(),
            ),
        )
            .into_response()),
        Err(error) => Err(error.into()),
    }
}

fn is_rule_refusal(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::Illegal(_)
            | StoreError::StaleBoard
            | StoreError::NotYourTurn
            | StoreError::GameFinished
            | StoreError::GameNotStarted
            | StoreError::NotASeat
            | StoreError::OpponentJoined
    )
}

/// Server-sent events: the page is told when something changes and reloads
/// itself. There is one stream for everything; a page filters by game id, and
/// the lobby (which has no id) reloads on any event.
///
/// Events carry only `<game id> <kind>` on purpose. Sending the new HTML or a
/// diff would mean the server keeping per-connection state about what the client
/// has already applied; a reload is cheap and cannot go out of sync.
async fn stream(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.events.subscribe();

    // Tell the client the pipe is open, so a reconnect can be told apart from
    // the first connection: on reconnect the page may have missed events, so it
    // reloads.
    let hello = tokio_stream::once(Ok(Event::default().event("hello").data("")));

    let updates = BroadcastStream::new(receiver).map(|result| match result {
        Ok(event) => Ok(Event::default().event("game").data(event.payload())),
        // This subscriber fell behind and lost events: tell it to resync rather
        // than leave it quietly out of date.
        Err(_) => Ok(Event::default().event("resync").data("")),
    });

    // Graceful shutdown waits for open responses, and an event stream is an
    // open response. Without this, one idle browser tab would hold a restart
    // up for the whole systemd stop timeout. A once-a-second comment as a
    // tick, ignored by clients, lets the stream notice and end.
    let mut shutdown = state.shutdown.subscribe();
    let ticks = IntervalStream::new(tokio::time::interval(Duration::from_secs(1)))
        .map(|_| Ok(Event::default().comment("tick")));

    Sse::new(
        hello
            .chain(updates)
            .merge(ticks)
            .take_while(move |_| !*shutdown.borrow_and_update()),
    )
    .keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(25))
            .text("keep-alive"),
    )
}

/// Fallback for clients without `EventSource`: one row, two fields. Any change
/// means "reload". The live stream is the primary path.
async fn game_version(
    State(state): State<AppState>,
    Path(game_id): Path<i64>,
) -> Result<Response, AppError> {
    let game = store::game(&state.db, game_id).await?;
    let body = format!(
        "{{\"seq\":{},\"state\":\"{}\"}}",
        game.seq,
        game.state.as_str()
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response())
}

async fn game_sgf(
    State(state): State<AppState>,
    Path(game_id): Path<i64>,
) -> Result<Response, AppError> {
    let game = store::game(&state.db, game_id).await?;
    let moves = store::moves(&state.db, game_id).await?;
    let record: Vec<(Cell, Option<Point>)> = moves.iter().map(|m| (m.color, m.point)).collect();
    let body = sgf::build(
        game.black.as_deref().unwrap_or(""),
        game.white.as_deref().unwrap_or(""),
        &date_ymd(game.created_at),
        &record,
    );
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "application/x-go-sgf; charset=utf-8".to_owned(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"komi-{game_id}.sgf\""),
            ),
        ],
        body,
    )
        .into_response())
}

// -------------------------------------------------------------- accounts

async fn register_form(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    if current.user().is_some() {
        return Ok(Redirect::to("/me").into_response());
    }
    if !state.config.allow_registration {
        return Err(AppError::Forbidden(
            i18n::t(lang, crate::i18n::Text::RegistrationClosed).to_owned(),
        ));
    }
    Ok(Html(views::register_page(lang, None, "").into_string()).into_response())
}

#[derive(Deserialize)]
struct RegisterForm {
    username: String,
    password: String,
}

async fn register(
    State(state): State<AppState>,
    client: ClientIp,
    Form(form): Form<RegisterForm>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let now = now_unix();
    if !state.config.allow_registration {
        return Err(AppError::Forbidden(
            i18n::t(lang, crate::i18n::Text::RegistrationClosed).to_owned(),
        ));
    }
    let limit_key = format!("register:{}", client.0);
    if state.register_limiter.is_blocked(&limit_key, now) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Html(
                views::register_page(
                    lang,
                    Some(i18n::t(lang, crate::i18n::Text::TooManyAccounts)),
                    &form.username,
                )
                .into_string(),
            ),
        )
            .into_response());
    }

    let username = form.username.trim().to_owned();
    if let Some(problem) = auth::username_problem(&username) {
        let message = auth::problem_text(lang, problem);
        return Ok((
            StatusCode::BAD_REQUEST,
            Html(views::register_page(lang, Some(&message), &username).into_string()),
        )
            .into_response());
    }
    if let Some(problem) = auth::password_problem(&form.password) {
        let message = auth::problem_text(lang, problem);
        return Ok((
            StatusCode::BAD_REQUEST,
            Html(views::register_page(lang, Some(&message), &username).into_string()),
        )
            .into_response());
    }

    let password_hash = auth::hash_password(&state.hasher, &form.password).await?;
    let user = match store::create_user(&state.db, &username, &password_hash, now).await {
        Ok(user) => user,
        Err(StoreError::UsernameTaken) => {
            return Ok((
                StatusCode::CONFLICT,
                Html(
                    views::register_page(
                        lang,
                        Some(i18n::t(lang, crate::i18n::Text::NameTaken)),
                        &username,
                    )
                    .into_string(),
                ),
            )
                .into_response())
        }
        Err(error) => return Err(error.into()),
    };

    state.register_limiter.record(&limit_key, now);
    start_session(&state, &user, "/me", now).await
}

async fn login_form(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<NextQuery>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    if current.user().is_some() {
        return Ok(Redirect::to("/me").into_response());
    }
    Ok(Html(
        views::login_page(
            lang,
            &next_target(&query.next),
            None,
            "",
            state.config.allow_registration,
        )
        .into_string(),
    )
    .into_response())
}

#[derive(Deserialize, Default)]
struct NextQuery {
    #[serde(default)]
    next: String,
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
    #[serde(default)]
    next: String,
}

async fn login(
    State(state): State<AppState>,
    client: ClientIp,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let now = now_unix();
    let next = next_target(&form.next);
    let limit_key = format!("login:{}", client.0);
    if state.login_limiter.is_blocked(&limit_key, now) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Html(
                views::login_page(
                    lang,
                    &next,
                    Some(i18n::t(lang, crate::i18n::Text::TooManyAttempts)),
                    &form.username,
                    state.config.allow_registration,
                )
                .into_string(),
            ),
        )
            .into_response());
    }

    let record = store::user_by_username(&state.db, form.username.trim()).await?;
    let verified = match &record {
        Some(record) => {
            auth::verify_password(&state.hasher, &form.password, &record.password_hash).await?
        }
        None => false,
    };
    let Some(record) = record.filter(|_| verified) else {
        state.login_limiter.record(&limit_key, now);
        return Ok((
            StatusCode::UNAUTHORIZED,
            Html(
                views::login_page(
                    lang,
                    &next,
                    Some(i18n::t(lang, crate::i18n::Text::WrongCredentials)),
                    &form.username,
                    state.config.allow_registration,
                )
                .into_string(),
            ),
        )
            .into_response());
    };

    state.login_limiter.forget(&limit_key);
    start_session(&state, &record.user, &next, now).await
}

/// Create a session and hand the browser a cookie for it.
async fn start_session(
    state: &AppState,
    user: &store::User,
    location: &str,
    now: i64,
) -> Result<Response, AppError> {
    let token = auth::new_token();
    store::create_session(
        &state.db,
        &auth::token_hash(&token),
        user.id,
        now,
        state.config.session_ttl_seconds(),
    )
    .await?;
    Ok(redirect_with_cookie(
        location,
        auth::session_cookie(
            &token,
            state.config.secure_cookies,
            state.config.session_ttl_seconds(),
        ),
    ))
}

async fn logout(State(state): State<AppState>, current: CurrentUser) -> Result<Response, AppError> {
    if let Some(session) = current.0 {
        store::delete_session(&state.db, &session.token_hash).await?;
    }
    Ok(redirect_with_cookie(
        "/",
        auth::clear_session_cookie(state.config.secure_cookies),
    ))
}

#[derive(Deserialize, Default)]
struct AccountQuery {
    #[serde(default)]
    ok: Option<String>,
}

async fn account(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<AccountQuery>,
) -> Result<Html<String>, AppError> {
    let lang = i18n::current();
    let session = current.require()?;
    let now = now_unix();
    let games = store::games(
        &state.db,
        store::GameQuery {
            filter: store::GameFilter::Mine(session.user.id),
            sort: store::GameSort::Active,
            limit: 12,
            now,
            stale_after: None,
        },
    )
    .await?;
    let message = match query.ok.as_deref() {
        Some("password") => Some(crate::i18n::Text::PasswordChanged),
        Some("logout") => Some(crate::i18n::Text::LoggedOutEverywhere),
        _ => None,
    };
    Ok(Html(
        views::me_page(
            &current,
            &games,
            now_unix(),
            state.config.session.days,
            message,
            lang,
        )
        .into_string(),
    ))
}

#[derive(Deserialize)]
struct PasswordForm {
    current: String,
    password: String,
}

async fn change_password(
    State(state): State<AppState>,
    current: CurrentUser,
    Form(form): Form<PasswordForm>,
) -> Result<Response, AppError> {
    let lang = i18n::current();
    let session = current.require()?;
    let record = store::user_record(&state.db, session.user.id)
        .await?
        .ok_or(AppError::NotFound)?;
    if !auth::verify_password(&state.hasher, &form.current, &record.password_hash).await? {
        return Err(AppError::BadRequest(
            i18n::t(lang, crate::i18n::Text::WrongCurrentPassword).to_owned(),
        ));
    }
    if let Some(problem) = auth::password_problem(&form.password) {
        return Err(AppError::BadRequest(auth::problem_text(lang, problem)));
    }
    let password_hash = auth::hash_password(&state.hasher, &form.password).await?;
    store::set_password(&state.db, session.user.id, &password_hash).await?;
    store::delete_sessions_for_user(&state.db, session.user.id, Some(session.token_hash)).await?;
    Ok(Redirect::to("/me?ok=password").into_response())
}

async fn logout_everywhere(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Response, AppError> {
    let session = current.require()?;
    store::delete_sessions_for_user(&state.db, session.user.id, Some(session.token_hash)).await?;
    Ok(Redirect::to("/me?ok=logout").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_relative_redirect_targets_are_accepted() {
        assert_eq!(next_target("/g/12"), "/g/12");
        assert_eq!(next_target("  /me  "), "/me");
        assert_eq!(next_target("//evil.example"), "/");
        assert_eq!(next_target("https://evil.example"), "/");
        assert_eq!(next_target("/\\evil.example"), "/");
        assert_eq!(next_target(""), "/");
    }

    #[test]
    fn rule_refusals_are_client_errors_not_server_errors() {
        assert!(is_rule_refusal(&StoreError::Illegal(
            crate::go::RuleViolation::Superko
        )));
        assert!(is_rule_refusal(&StoreError::StaleBoard));
        assert!(!is_rule_refusal(&StoreError::Storage("boom".to_owned())));
    }
}
