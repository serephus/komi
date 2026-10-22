//! Application errors and how they become HTTP responses.
//!
//! Handlers that can describe a failure in the user's language do so and pass a
//! finished string (`AppError::BadRequest`). Errors that travel up from the
//! store carry no text at all; they are mapped to a [`Text`] here, and the
//! language comes from the task-local that the language middleware sets around
//! each request — the generic error page is rendered outside any handler, so it
//! has no other way to learn it.

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};

use crate::i18n::{self, Lang, Text};
use crate::store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Store(#[from] StoreError),
    #[error("that page does not exist")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("you need to log in first")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::Store(error) => match error {
                StoreError::GameNotFound => StatusCode::NOT_FOUND,
                StoreError::UsernameTaken => StatusCode::CONFLICT,
                StoreError::Storage(_) | StoreError::Corrupt(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
                _ => StatusCode::BAD_REQUEST,
            },
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Heading for the page, and the body text.
    fn texts(&self, lang: Lang) -> (Text, String) {
        let t = i18n::translator(lang);
        match self {
            AppError::Store(error) => (Text::TitleRefused, t(store_text(error)).to_owned()),
            AppError::NotFound => (Text::TitleNotFound, t(Text::MessageNotFound).to_owned()),
            AppError::Unauthorized => (
                Text::TitleNotLoggedIn,
                t(Text::MessageNotLoggedIn).to_owned(),
            ),
            AppError::BadRequest(message) | AppError::Forbidden(message) => {
                (Text::TitleRefused, message.clone())
            }
            AppError::Internal(_) => (Text::TitleUnexpected, t(Text::MessageUnexpected).to_owned()),
        }
    }
}

/// The translated message for a store failure. Internal failures are not
/// described to the visitor; they are logged instead.
pub fn store_text(error: &StoreError) -> Text {
    match error {
        StoreError::Storage(_) | StoreError::Corrupt(_) => Text::ErrUnavailable,
        StoreError::UsernameTaken => Text::NameTaken,
        StoreError::GameNotFound => Text::ErrGameNotFound,
        StoreError::NotASeat => Text::ErrNotASeat,
        StoreError::NotYourTurn => Text::ErrNotYourTurn,
        StoreError::GameNotStarted => Text::ErrGameNotStarted,
        StoreError::SeatAlreadyTaken => Text::ErrSeatTaken,
        StoreError::OwnGame => Text::ErrOwnGame,
        StoreError::GameFinished => Text::ErrGameFinished,
        StoreError::StaleBoard => Text::ErrStaleBoard,
        StoreError::OpponentJoined => Text::ErrOpponentJoined,
        StoreError::Illegal(violation) => match violation {
            crate::go::RuleViolation::Occupied => Text::ErrOccupied,
            crate::go::RuleViolation::Suicide => Text::ErrSuicide,
            crate::go::RuleViolation::Superko => Text::ErrSuperko,
        },
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let lang = i18n::current();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = %self, "request failed");
        }
        let (title, message) = self.texts(lang);
        let body = crate::web::views::error_page(lang, status, title, &message).into_string();
        (status, Html(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::go::RuleViolation;

    #[test]
    fn statuses_are_mapped() {
        assert_eq!(AppError::NotFound.status(), StatusCode::NOT_FOUND);
        assert_eq!(AppError::Unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            AppError::Store(StoreError::GameNotFound).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppError::Store(StoreError::Illegal(RuleViolation::Superko)).status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            AppError::Internal("boom".to_owned()).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn every_store_error_has_a_translated_message() {
        for error in [
            StoreError::GameNotFound,
            StoreError::NotASeat,
            StoreError::NotYourTurn,
            StoreError::GameNotStarted,
            StoreError::SeatAlreadyTaken,
            StoreError::OwnGame,
            StoreError::GameFinished,
            StoreError::StaleBoard,
            StoreError::OpponentJoined,
            StoreError::UsernameTaken,
            StoreError::Illegal(RuleViolation::Occupied),
            StoreError::Illegal(RuleViolation::Suicide),
            StoreError::Illegal(RuleViolation::Superko),
        ] {
            let key = store_text(&error);
            for lang in Lang::ALL {
                assert!(!i18n::t(lang, key).is_empty());
            }
        }
    }

    #[test]
    fn internal_failures_are_not_described() {
        let (title, message) = AppError::Internal("database on fire".to_owned()).texts(Lang::En);
        assert_eq!(title, Text::TitleUnexpected);
        assert!(!message.contains("database on fire"));
    }

    #[test]
    fn handler_messages_are_passed_through_already_translated() {
        let error = AppError::BadRequest("棋盘变了".to_owned());
        let (_, message) = error.texts(Lang::Zh);
        assert_eq!(message, "棋盘变了");
    }
}
