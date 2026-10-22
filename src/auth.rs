//! Identity: password hashing, session cookies, rate limiting and the
//! `CurrentUser` extractor.
//!
//! There is no email address and no password reset: an account is a username, a
//! password (Argon2id) and a server side session. Losing the password loses the
//! account, which is why the account page shows how long the session lasts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::header::{COOKIE, HOST, ORIGIN};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use base64ct::{Base64UrlUnpadded, Encoding};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::error::AppError;
use crate::i18n::{self, Lang, Text};
use crate::store::{self, User};
use crate::util::now_unix;
use crate::web::AppState;

pub const SESSION_COOKIE: &str = "komi_session";
pub const MIN_PASSWORD_LENGTH: usize = 10;
pub const MAX_PASSWORD_LENGTH: usize = 200;
pub const MIN_USERNAME_LENGTH: usize = 3;
pub const MAX_USERNAME_LENGTH: usize = 32;

/// Refresh the session's expiry at most this often, so that polling the game
/// page does not turn into a write every ten seconds.
const SESSION_TOUCH_INTERVAL: i64 = 60 * 60;

// ------------------------------------------------------------- passwords

fn hash_blocking(password: &str) -> Result<String, AppError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|error| AppError::Internal(format!("could not hash password: {error}")))
}

fn verify_blocking(password: &str, stored: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), stored)
        .is_ok()
}

/// Argon2id is memory hard, so only a few may run at once or a burst of login
/// attempts could exhaust the process.
pub async fn hash_password(hasher: &Arc<Semaphore>, password: &str) -> Result<String, AppError> {
    let password = password.to_owned();
    let _permit = hasher
        .acquire()
        .await
        .map_err(|_| AppError::Internal("password hasher is closing".to_owned()))?;
    tokio::task::spawn_blocking(move || hash_blocking(&password))
        .await
        .map_err(|error| AppError::Internal(format!("password hashing failed: {error}")))?
}

pub async fn verify_password(
    hasher: &Arc<Semaphore>,
    password: &str,
    stored: &str,
) -> Result<bool, AppError> {
    let password = password.to_owned();
    let stored = stored.to_owned();
    let _permit = hasher
        .acquire()
        .await
        .map_err(|_| AppError::Internal("password hasher is closing".to_owned()))?;
    tokio::task::spawn_blocking(move || verify_blocking(&password, &stored))
        .await
        .map_err(|error| AppError::Internal(format!("password verification failed: {error}")))
}

/// Returns the problem with a password, if any, as a message key.
pub fn password_problem(password: &str) -> Option<Text> {
    if password.len() < MIN_PASSWORD_LENGTH {
        Some(Text::PasswordTooShort)
    } else if password.len() > MAX_PASSWORD_LENGTH {
        Some(Text::PasswordTooLong)
    } else {
        None
    }
}

/// Returns the problem with a username, if any, as a message key.
pub fn username_problem(username: &str) -> Option<Text> {
    let length = username.chars().count();
    if length < MIN_USERNAME_LENGTH {
        return Some(Text::NameTooShort);
    }
    if length > MAX_USERNAME_LENGTH {
        return Some(Text::NameTooLong);
    }
    if !username
        .chars()
        .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
    {
        return Some(Text::NameCharacters);
    }
    if username
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit() || ch == '_' || ch == '-' || ch == '.')
    {
        return Some(Text::NameStart);
    }
    None
}

/// The rule, in words, with its limit filled in.
pub fn problem_text(lang: Lang, problem: Text) -> String {
    let limit = match problem {
        Text::PasswordTooShort => MIN_PASSWORD_LENGTH,
        Text::PasswordTooLong => MAX_PASSWORD_LENGTH,
        Text::NameTooShort => MIN_USERNAME_LENGTH,
        Text::NameTooLong => MAX_USERNAME_LENGTH,
        _ => {
            return i18n::t(lang, problem).to_owned();
        }
    };
    i18n::fill(i18n::t(lang, problem), &[("count", &limit.to_string())])
}

// ---------------------------------------------------------------- tokens

/// A 32 byte secret, base64url encoded. Used for session cookies.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system must provide randomness");
    Base64UrlUnpadded::encode_string(&bytes)
}

/// Tokens are stored hashed, so a database dump does not hand out live sessions.
pub fn token_hash(token: &str) -> [u8; 32] {
    let digest = Sha256::digest(token.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

// --------------------------------------------------------------- cookies

pub fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key.trim() == name).then(|| value.trim().to_owned())
        })
        .next()
}

pub fn session_cookie(token: &str, secure: bool, max_age_seconds: i64) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_seconds}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn clear_session_cookie(secure: bool) -> String {
    let mut cookie = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

// ---------------------------------------------------- cross site requests

/// Reject state changing requests that a browser says came from another site.
///
/// `SameSite=Lax` already covers most of this; the header checks are the second
/// lock on the same door. Non-browser clients that send neither header are
/// allowed, which keeps `curl` usable against a self-hosted instance.
pub fn same_origin(headers: &HeaderMap) -> bool {
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        return matches!(site, "same-origin" | "none");
    }
    match (
        headers.get(ORIGIN).and_then(|v| v.to_str().ok()),
        headers.get(HOST).and_then(|v| v.to_str().ok()),
    ) {
        (Some(origin), Some(host)) => {
            let without_scheme = origin
                .split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(origin);
            without_scheme.eq_ignore_ascii_case(host)
        }
        (None, _) => true,
        (Some(_), None) => false,
    }
}

// ---------------------------------------------------------- rate limiting

/// Fixed window counter, in memory. Only used to slow down password guessing on
/// a single instance, which is the right size for a self-hosted game server.
#[derive(Debug)]
pub struct RateLimiter {
    limit: u32,
    window: i64,
    counts: Mutex<HashMap<String, (u32, i64)>>,
}

impl RateLimiter {
    pub fn new(limit: u32, window_seconds: i64) -> Self {
        RateLimiter {
            limit,
            window: window_seconds,
            counts: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_blocked(&self, key: &str, now: i64) -> bool {
        let counts = self.counts.lock().expect("rate limiter lock");
        counts
            .get(key)
            .is_some_and(|(count, reset_at)| *reset_at > now && *count >= self.limit)
    }

    pub fn record(&self, key: &str, now: i64) {
        let mut counts = self.counts.lock().expect("rate limiter lock");
        if counts.len() > 10_000 {
            counts.retain(|_, (_, reset_at)| *reset_at > now);
        }
        let entry = counts
            .entry(key.to_owned())
            .or_insert((0, now + self.window));
        if entry.1 <= now {
            *entry = (1, now + self.window);
        } else {
            entry.0 = entry.0.saturating_add(1);
        }
    }

    pub fn forget(&self, key: &str) {
        self.counts.lock().expect("rate limiter lock").remove(key);
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        RateLimiter::new(10, 15 * 60)
    }
}

// ------------------------------------------------------------- extractors

#[derive(Debug, Clone)]
pub struct Session {
    pub user: User,
    pub token_hash: [u8; 32],
}

/// The logged in user, if any. Never fails: handlers that need a login ask for
/// it explicitly so that the failure is visible at the call site.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub Option<Session>);

impl CurrentUser {
    pub fn user(&self) -> Option<&User> {
        self.0.as_ref().map(|session| &session.user)
    }

    pub fn id(&self) -> Option<i64> {
        self.user().map(|user| user.id)
    }

    /// Use in handlers that cannot work anonymously.
    pub fn require(&self) -> Result<&Session, AppError> {
        self.0.as_ref().ok_or(AppError::Unauthorized)
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(token) = read_cookie(&parts.headers, SESSION_COOKIE) else {
            return Ok(CurrentUser(None));
        };
        let token_hash = token_hash(&token);
        let now = now_unix();
        let Some((user, last_seen_at)) =
            store::user_for_session(&state.db, &token_hash, now).await?
        else {
            return Ok(CurrentUser(None));
        };
        if now - last_seen_at > SESSION_TOUCH_INTERVAL {
            store::touch_session(
                &state.db,
                &token_hash,
                now,
                state.config.session_ttl_seconds(),
            )
            .await?;
        }
        Ok(CurrentUser(Some(Session { user, token_hash })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::HeaderValue;

    #[tokio::test]
    async fn passwords_round_trip_through_argon2id() {
        let hasher = Arc::new(Semaphore::new(2));
        let hash = hash_password(&hasher, "correct horse battery")
            .await
            .unwrap();
        assert!(hash.starts_with("$argon2id$"), "got {hash}");
        assert!(verify_password(&hasher, "correct horse battery", &hash)
            .await
            .unwrap());
        assert!(!verify_password(&hasher, "wrong horse battery", &hash)
            .await
            .unwrap());
        assert!(!verify_password(&hasher, "", &hash).await.unwrap());
    }

    #[tokio::test]
    async fn every_hash_gets_its_own_salt() {
        let hasher = Arc::new(Semaphore::new(2));
        let first = hash_password(&hasher, "same password here").await.unwrap();
        let second = hash_password(&hasher, "same password here").await.unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn passwords_must_be_long_enough_to_be_worth_hashing() {
        assert!(password_problem("short").is_some());
        assert!(password_problem("0123456789").is_none());
        assert!(password_problem(&"x".repeat(MAX_PASSWORD_LENGTH + 1)).is_some());
    }

    #[test]
    fn usernames_are_restricted_to_something_linkable() {
        assert!(username_problem("alice").is_none());
        assert!(username_problem("a.b-c_1").is_none());
        assert!(username_problem("ab").is_some());
        assert!(username_problem(&"x".repeat(MAX_USERNAME_LENGTH + 1)).is_some());
        assert!(username_problem("has space").is_some());
        assert!(username_problem("_leading").is_some());
        assert!(username_problem("").is_some());
    }

    #[test]
    fn tokens_are_long_and_unique() {
        let first = new_token();
        let second = new_token();
        assert_ne!(first, second);
        assert!(first.len() >= 42, "got {} chars", first.len());
        assert_eq!(token_hash(&first), token_hash(&first));
        assert_ne!(token_hash(&first), token_hash(&second));
    }

    #[test]
    fn cookies_are_read_from_the_cookie_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("other=1; komi_session=abc123; trailing=2"),
        );
        assert_eq!(
            read_cookie(&headers, SESSION_COOKIE).as_deref(),
            Some("abc123")
        );
        assert_eq!(read_cookie(&headers, "missing"), None);

        let mut empty = HeaderMap::new();
        empty.insert(COOKIE, HeaderValue::from_static(""));
        assert_eq!(read_cookie(&empty, SESSION_COOKIE), None);
    }

    #[test]
    fn session_cookies_are_hardened() {
        let cookie = session_cookie("token", true, 60);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("Max-Age=60"));
        assert!(!session_cookie("token", false, 60).contains("Secure"));
        assert!(clear_session_cookie(false).contains("Max-Age=0"));
    }

    #[test]
    fn cross_site_posts_are_rejected() {
        let mut headers = HeaderMap::new();
        assert!(same_origin(&headers), "no headers: allow (curl, tests)");

        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(same_origin(&headers));
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(!same_origin(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("go.example.org"));
        headers.insert(ORIGIN, HeaderValue::from_static("https://go.example.org"));
        assert!(same_origin(&headers));
        headers.insert(ORIGIN, HeaderValue::from_static("https://evil.example.org"));
        assert!(!same_origin(&headers));
    }

    #[test]
    fn the_rate_limiter_forgets_old_attempts() {
        let limiter = RateLimiter::new(2, 100);
        let now = 1_000;
        limiter.record("ip", now);
        limiter.record("ip", now);
        assert!(limiter.is_blocked("ip", now));
        assert!(!limiter.is_blocked("other", now));
        // The window expires.
        assert!(!limiter.is_blocked("ip", now + 101));
        limiter.record("ip", now + 101);
        assert!(!limiter.is_blocked("ip", now + 101));

        limiter.record("ip", now + 101);
        assert!(limiter.is_blocked("ip", now + 101));
        limiter.forget("ip");
        assert!(!limiter.is_blocked("ip", now + 101));
    }
}
