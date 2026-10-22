//! Every read and write of the game data.
//!
//! The store is a redb database; see [`crate::db`] for the tables and for what
//! redb no longer enforces on our behalf. Two rules hold throughout:
//!
//! * anything that must be atomic — a move, a claim, a withdrawal, a login —
//!   happens in one write transaction, which redb allows only one of at a time;
//! * reads take a read transaction, so they never block the writer and never see
//!   a half-finished change.

use std::cmp::Reverse;
use std::collections::HashSet;

use redb::{ReadableDatabase, ReadableTable};

use crate::db::{
    self, storage, GameRow, MoveRow, Store, UserRow, GAMES, META, MOVES, NEXT_GAME, NEXT_USER,
    SESSIONS, USERNAMES, USERS,
};
use crate::go::rules::Positions;
use crate::go::{Board, Cell, Point, RuleViolation};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("stored data is invalid: {0}")]
    Corrupt(&'static str),
    #[error("that username is already taken")]
    UsernameTaken,
    #[error("game not found")]
    GameNotFound,
    #[error("you are not one of the two players in this game")]
    NotASeat,
    #[error("it is not your turn")]
    NotYourTurn,
    #[error("this game is still waiting for an opponent")]
    GameNotStarted,
    #[error("this game already has an opponent")]
    SeatAlreadyTaken,
    #[error("you cannot take the seat in your own game")]
    OwnGame,
    #[error("this game is finished")]
    GameFinished,
    #[error("the board changed while you were looking at it")]
    StaleBoard,
    #[error("this game already has an opponent, so it cannot be withdrawn")]
    OpponentJoined,
    #[error("{0}")]
    Illegal(#[from] RuleViolation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub user: User,
    pub password_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    Waiting,
    Active,
    Finished,
}

impl GameState {
    pub fn as_str(self) -> &'static str {
        match self {
            GameState::Waiting => "waiting",
            GameState::Active => "active",
            GameState::Finished => "finished",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Game {
    pub id: i64,
    /// Either seat may be empty: a game can exist before an opponent does.
    pub black_id: Option<i64>,
    pub white_id: Option<i64>,
    pub black: Option<String>,
    pub white: Option<String>,
    pub state: GameState,
    /// Number of moves played so far. Also the optimistic-concurrency token the
    /// browser has to echo back when it submits a move.
    pub seq: u32,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
    /// The current position, filled in by the listing queries and [`game`].
    pub position: Option<Board>,
    /// The last stone played, for the marker on a preview board.
    pub last_move: Option<Point>,
}

impl Game {
    /// Black moves on even move numbers, white on odd ones.
    pub fn turn(&self) -> Cell {
        if self.seq.is_multiple_of(2) {
            Cell::Black
        } else {
            Cell::White
        }
    }

    /// The user in a seat, if anyone is sitting there.
    pub fn seat(&self, colour: Cell) -> Option<i64> {
        match colour {
            Cell::Black => self.black_id,
            Cell::White => self.white_id,
            Cell::Empty => None,
        }
    }

    /// The empty seat, if the game is still looking for a player.
    pub fn open_seat(&self) -> Option<Cell> {
        match (self.black_id, self.white_id) {
            (None, _) => Some(Cell::Black),
            (_, None) => Some(Cell::White),
            _ => None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open_seat().is_some()
    }

    /// Which colour `user_id` plays, if any.
    pub fn seat_of(&self, user_id: i64) -> Option<Cell> {
        if self.black_id == Some(user_id) {
            Some(Cell::Black)
        } else if self.white_id == Some(user_id) {
            Some(Cell::White)
        } else {
            None
        }
    }

    /// The name in a seat, or `None` when nobody is sitting there. Naming an
    /// empty seat is the view's job, since it has to be translated.
    pub fn player(&self, colour: Cell) -> Option<&str> {
        match colour {
            Cell::Black => self.black.as_deref(),
            Cell::White => self.white.as_deref(),
            Cell::Empty => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Move {
    pub seq: u32,
    pub color: Cell,
    pub point: Option<Point>,
    pub played_at: i64,
}

#[derive(Debug, Clone)]
pub struct PlayOutcome {
    pub seq: u32,
    pub state: GameState,
    pub captured: Vec<Point>,
}

/// The state follows from the seats and the ending; it is never stored, so it
/// cannot disagree with them.
fn state_of(black: Option<u64>, white: Option<u64>, finished_at: Option<i64>) -> GameState {
    if finished_at.is_some() {
        GameState::Finished
    } else if black.is_some() && white.is_some() {
        GameState::Active
    } else {
        GameState::Waiting
    }
}

fn key(id: i64) -> u64 {
    id as u64
}

fn id_of(raw: u64) -> i64 {
    raw as i64
}

/// Where the moves of one game live in the key space.
fn move_keys(game_id: u64) -> impl std::ops::RangeBounds<(u64, u32)> {
    (game_id, 0)..=(game_id, u32::MAX)
}

fn colour_of(value: &str) -> Result<Cell, StoreError> {
    match value {
        "black" => Ok(Cell::Black),
        "white" => Ok(Cell::White),
        _ => Err(StoreError::Corrupt("move colour")),
    }
}

/// The position after the last move: one lookup, because the game knows its own
/// move number.
fn position_at(
    moves: &impl ReadableTable<(u64, u32), MoveRow>,
    game_id: u64,
    seq: u32,
) -> Result<Option<Board>, StoreError> {
    if seq == 0 {
        return Ok(None);
    }
    let bytes = moves
        .get((game_id, seq))
        .map_err(storage)?
        .map(|guard| guard.value().2.to_vec());
    Ok(bytes.as_deref().and_then(Board::from_bytes))
}

/// The last point actually played: a pass is not a place on the board, so walk
/// back over any trailing passes.
fn last_move_at(
    moves: &impl ReadableTable<(u64, u32), MoveRow>,
    game_id: u64,
    seq: u32,
) -> Result<Option<Point>, StoreError> {
    let mut at = seq;
    while at > 0 {
        let text = moves
            .get((game_id, at))
            .map_err(storage)?
            .and_then(|guard| guard.value().1.map(str::to_owned));
        if let Some(text) = text {
            if let Some(point) = Point::parse_label(&text) {
                return Ok(Some(point));
            }
        }
        at -= 1;
    }
    Ok(None)
}

fn username_of(
    users: &impl ReadableTable<u64, UserRow>,
    id: Option<u64>,
) -> Result<Option<String>, StoreError> {
    let Some(id) = id else {
        return Ok(None);
    };
    Ok(users
        .get(id)
        .map_err(storage)?
        .map(|guard| guard.value().0.to_owned()))
}

fn build_game(
    id: u64,
    row: GameRow,
    users: &impl ReadableTable<u64, UserRow>,
    moves: &impl ReadableTable<(u64, u32), MoveRow>,
) -> Result<Game, StoreError> {
    let (black, white, seq, created_at, updated_at, finished_at) = row;
    Ok(Game {
        id: id_of(id),
        black_id: black.map(id_of),
        white_id: white.map(id_of),
        black: username_of(users, black)?,
        white: username_of(users, white)?,
        state: state_of(black, white, finished_at),
        seq,
        created_at,
        updated_at,
        finished_at,
        position: position_at(moves, id, seq)?,
        last_move: last_move_at(moves, id, seq)?,
    })
}

// ---------------------------------------------------------------- users

pub async fn create_user(
    store: &Store,
    username: &str,
    password_hash: &str,
    now: i64,
) -> Result<User, StoreError> {
    let username = username.to_owned();
    let password_hash = password_hash.to_owned();
    store
        .blocking(move |db| {
            let lower = username.to_lowercase();
            let write = db.begin_write().map_err(storage)?;
            let id = {
                let mut meta = write.open_table(META).map_err(storage)?;
                let mut names = write.open_table(USERNAMES).map_err(storage)?;
                if names.get(lower.as_str()).map_err(storage)?.is_some() {
                    return Err(StoreError::UsernameTaken);
                }
                let id = db::next_id(&mut meta, NEXT_USER)?;
                names.insert(lower.as_str(), id).map_err(storage)?;
                let mut users = write.open_table(USERS).map_err(storage)?;
                users
                    .insert(
                        id,
                        (
                            username.as_str(),
                            lower.as_str(),
                            password_hash.as_str(),
                            now,
                        ),
                    )
                    .map_err(storage)?;
                id
            };
            write.commit().map_err(storage)?;
            Ok(User {
                id: id_of(id),
                username,
            })
        })
        .await
}

pub async fn user_by_username(
    store: &Store,
    username: &str,
) -> Result<Option<UserRecord>, StoreError> {
    let lower = username.to_lowercase();
    store
        .blocking(move |db| {
            let read = db.begin_read().map_err(storage)?;
            let names = read.open_table(USERNAMES).map_err(storage)?;
            let Some(id) = names
                .get(lower.as_str())
                .map_err(storage)?
                .map(|guard| guard.value())
            else {
                return Ok(None);
            };
            record_of(&read, id)
        })
        .await
}

pub async fn user_by_id(store: &Store, user_id: i64) -> Result<Option<User>, StoreError> {
    store
        .blocking(move |db| {
            let read = db.begin_read().map_err(storage)?;
            Ok(record_of(&read, key(user_id))?.map(|record| record.user))
        })
        .await
}

pub async fn user_record(store: &Store, user_id: i64) -> Result<Option<UserRecord>, StoreError> {
    store
        .blocking(move |db| {
            let read = db.begin_read().map_err(storage)?;
            record_of(&read, key(user_id))
        })
        .await
}

fn record_of(read: &redb::ReadTransaction, id: u64) -> Result<Option<UserRecord>, StoreError> {
    let users = read.open_table(USERS).map_err(storage)?;
    let Some(guard) = users.get(id).map_err(storage)? else {
        return Ok(None);
    };
    let (username, _, password_hash, _) = guard.value();
    Ok(Some(UserRecord {
        user: User {
            id: id_of(id),
            username: username.to_owned(),
        },
        password_hash: password_hash.to_owned(),
    }))
}

pub async fn set_password(
    store: &Store,
    user_id: i64,
    password_hash: &str,
) -> Result<(), StoreError> {
    let password_hash = password_hash.to_owned();
    store
        .blocking(move |db| {
            let write = db.begin_write().map_err(storage)?;
            {
                let mut users = write.open_table(USERS).map_err(storage)?;
                let (username, lower, created_at) = {
                    let guard = users
                        .get(key(user_id))
                        .map_err(storage)?
                        .ok_or(StoreError::GameNotFound)?;
                    let (username, lower, _, created_at) = guard.value();
                    (username.to_owned(), lower.to_owned(), created_at)
                };
                users
                    .insert(
                        key(user_id),
                        (
                            username.as_str(),
                            lower.as_str(),
                            password_hash.as_str(),
                            created_at,
                        ),
                    )
                    .map_err(storage)?;
            }
            write.commit().map_err(storage)?;
            Ok(())
        })
        .await
}

// ------------------------------------------------------------- sessions

pub async fn create_session(
    store: &Store,
    token_hash: &[u8; 32],
    user_id: i64,
    now: i64,
    ttl_seconds: i64,
) -> Result<(), StoreError> {
    let token = token_hash.to_vec();
    store
        .blocking(move |db| {
            let write = db.begin_write().map_err(storage)?;
            {
                let mut sessions = write.open_table(SESSIONS).map_err(storage)?;
                // Opportunistic cleanup: sessions are tiny, but they are the
                // only table that grows from ordinary use.
                sessions
                    .retain(|_, (_, _, _, expires_at)| expires_at > now)
                    .map_err(storage)?;
                sessions
                    .insert(
                        token.as_slice(),
                        (key(user_id), now, now, now + ttl_seconds),
                    )
                    .map_err(storage)?;
            }
            write.commit().map_err(storage)?;
            Ok(())
        })
        .await
}

pub async fn user_for_session(
    store: &Store,
    token_hash: &[u8; 32],
    now: i64,
) -> Result<Option<(User, i64)>, StoreError> {
    let token = token_hash.to_vec();
    store
        .blocking(move |db| {
            let read = db.begin_read().map_err(storage)?;
            let sessions = read.open_table(SESSIONS).map_err(storage)?;
            let Some(guard) = sessions.get(token.as_slice()).map_err(storage)? else {
                return Ok(None);
            };
            let (user_id, _, last_seen_at, expires_at) = guard.value();
            drop(guard);
            if expires_at <= now {
                return Ok(None);
            }
            let Some(record) = record_of(&read, user_id)? else {
                return Ok(None);
            };
            Ok(Some((record.user, last_seen_at)))
        })
        .await
}

pub async fn touch_session(
    store: &Store,
    token_hash: &[u8; 32],
    now: i64,
    ttl_seconds: i64,
) -> Result<(), StoreError> {
    let token = token_hash.to_vec();
    store
        .blocking(move |db| {
            let write = db.begin_write().map_err(storage)?;
            {
                let mut sessions = write.open_table(SESSIONS).map_err(storage)?;
                let row = {
                    let guard = sessions.get(token.as_slice()).map_err(storage)?;
                    guard.map(|guard| {
                        let (user_id, created_at, _, _) = guard.value();
                        (user_id, created_at)
                    })
                };
                if let Some((user_id, created_at)) = row {
                    sessions
                        .insert(
                            token.as_slice(),
                            (user_id, created_at, now, now + ttl_seconds),
                        )
                        .map_err(storage)?;
                }
            }
            write.commit().map_err(storage)?;
            Ok(())
        })
        .await
}

pub async fn delete_session(store: &Store, token_hash: &[u8; 32]) -> Result<(), StoreError> {
    let token = token_hash.to_vec();
    store
        .blocking(move |db| {
            let write = db.begin_write().map_err(storage)?;
            {
                let mut sessions = write.open_table(SESSIONS).map_err(storage)?;
                sessions.remove(token.as_slice()).map_err(storage)?;
            }
            write.commit().map_err(storage)?;
            Ok(())
        })
        .await
}

/// Log out of every device, optionally keeping the session that is asking.
pub async fn delete_sessions_for_user(
    store: &Store,
    user_id: i64,
    keep: Option<[u8; 32]>,
) -> Result<(), StoreError> {
    let keep = keep.map(|token| token.to_vec());
    store
        .blocking(move |db| {
            let write = db.begin_write().map_err(storage)?;
            {
                let mut sessions = write.open_table(SESSIONS).map_err(storage)?;
                sessions
                    .retain(|token, (owner, _, _, _)| {
                        owner != key(user_id) || keep.as_deref() == Some(token)
                    })
                    .map_err(storage)?;
            }
            write.commit().map_err(storage)?;
            Ok(())
        })
        .await
}

// ---------------------------------------------------------------- games

pub async fn create_game(
    store: &Store,
    creator_id: i64,
    creator_colour: Cell,
    opponent: Option<&User>,
    now: i64,
) -> Result<i64, StoreError> {
    let opponent_id = opponent.map(|user| key(user.id));
    let (black, white) = match creator_colour {
        Cell::Black => (Some(key(creator_id)), opponent_id),
        Cell::White => (opponent_id, Some(key(creator_id))),
        Cell::Empty => return Err(StoreError::Corrupt("creator colour")),
    };
    store
        .blocking(move |db| {
            let write = db.begin_write().map_err(storage)?;
            let id = {
                let mut meta = write.open_table(META).map_err(storage)?;
                let id = db::next_id(&mut meta, NEXT_GAME)?;
                let mut games = write.open_table(GAMES).map_err(storage)?;
                games
                    .insert(id, (black, white, 0, now, now, None))
                    .map_err(storage)?;
                id
            };
            write.commit().map_err(storage)?;
            Ok(id_of(id))
        })
        .await
}

/// Sit down in whichever seat is still empty. Used for the lobby's "take the
/// seat" button, and for joining a game whose owner has already moved.
pub async fn claim_seat(
    store: &Store,
    game_id: i64,
    user_id: i64,
    now: i64,
) -> Result<(), StoreError> {
    store
        .blocking(move |db| {
            let write = db.begin_write().map_err(storage)?;
            {
                let mut games = write.open_table(GAMES).map_err(storage)?;
                let row = {
                    let guard = games
                        .get(key(game_id))
                        .map_err(storage)?
                        .ok_or(StoreError::GameNotFound)?;
                    let (black, white, seq, created_at, _, finished_at) = guard.value();
                    (black, white, seq, created_at, finished_at)
                };
                let (black, white, seq, created_at, finished_at) = row;
                if black == Some(key(user_id)) || white == Some(key(user_id)) {
                    return Err(StoreError::OwnGame);
                }
                let (black, white) = match (black, white) {
                    (None, white) => (Some(key(user_id)), white),
                    (black, None) => (black, Some(key(user_id))),
                    _ => return Err(StoreError::SeatAlreadyTaken),
                };
                games
                    .insert(
                        key(game_id),
                        (black, white, seq, created_at, now, finished_at),
                    )
                    .map_err(storage)?;
            }
            write.commit().map_err(storage)?;
            Ok(())
        })
        .await
}

/// Abandon an open game before anyone takes the free seat: the moves go with it,
/// because nothing else was written there.
pub async fn withdraw_game(store: &Store, game_id: i64, user_id: i64) -> Result<(), StoreError> {
    store
        .blocking(move |db| {
            let write = db.begin_write().map_err(storage)?;
            {
                let mut games = write.open_table(GAMES).map_err(storage)?;
                let row = {
                    let guard = games
                        .get(key(game_id))
                        .map_err(storage)?
                        .ok_or(StoreError::GameNotFound)?;
                    let (black, white, _, _, _, _) = guard.value();
                    (black, white)
                };
                let (black, white) = row;
                if black != Some(key(user_id)) && white != Some(key(user_id)) {
                    return Err(StoreError::NotASeat);
                }
                if black.is_some() && white.is_some() {
                    return Err(StoreError::OpponentJoined);
                }
                let mut moves = write.open_table(MOVES).map_err(storage)?;
                moves
                    .retain_in(move_keys(key(game_id)), |_, _| false)
                    .map_err(storage)?;
                drop(moves);
                games.remove(key(game_id)).map_err(storage)?;
            }
            write.commit().map_err(storage)?;
            Ok(())
        })
        .await
}

pub async fn game(store: &Store, game_id: i64) -> Result<Game, StoreError> {
    store
        .blocking(move |db| {
            let read = db.begin_read().map_err(storage)?;
            let games = read.open_table(GAMES).map_err(storage)?;
            let row = {
                let guard = games
                    .get(key(game_id))
                    .map_err(storage)?
                    .ok_or(StoreError::GameNotFound)?;
                let (black, white, seq, created_at, updated_at, finished_at) = guard.value();
                (black, white, seq, created_at, updated_at, finished_at)
            };
            let users = read.open_table(USERS).map_err(storage)?;
            let moves = read.open_table(MOVES).map_err(storage)?;
            build_game(key(game_id), row, &users, &moves)
        })
        .await
}

/// Which games the lobby is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GameFilter {
    #[default]
    All,
    /// A seat is still free.
    Open,
    /// Both seats taken, not finished.
    Running,
    Finished,
    /// Games the given user is playing in.
    Mine(i64),
}

impl GameFilter {
    /// Parse the `?filter=` value. Anything unknown falls back to the default.
    pub fn parse(value: &str, viewer: Option<i64>) -> Self {
        match value {
            "open" => GameFilter::Open,
            "running" => GameFilter::Running,
            "finished" => GameFilter::Finished,
            "mine" => match viewer {
                Some(user_id) => GameFilter::Mine(user_id),
                None => GameFilter::All,
            },
            _ => GameFilter::All,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            GameFilter::All => "all",
            GameFilter::Open => "open",
            GameFilter::Running => "running",
            GameFilter::Finished => "finished",
            GameFilter::Mine(_) => "mine",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GameSort {
    /// Most recently played first.
    #[default]
    Active,
    /// Newest game first.
    Newest,
    /// Quietest game first: the way to find something that went to sleep.
    Oldest,
}

impl GameSort {
    pub fn parse(value: &str) -> Self {
        match value {
            "newest" => GameSort::Newest,
            "oldest" => GameSort::Oldest,
            _ => GameSort::Active,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            GameSort::Active => "active",
            GameSort::Newest => "newest",
            GameSort::Oldest => "oldest",
        }
    }
}

/// One page of games, with the position each one is at.
pub struct GameQuery {
    pub filter: GameFilter,
    pub sort: GameSort,
    pub limit: i64,
    pub now: i64,
    /// Games with a free seat that nobody has touched for this long are left out
    /// (except from somebody's own list): a free seat is an advertisement, and
    /// month-old adverts are noise. `None` keeps them.
    pub stale_after: Option<i64>,
}

pub async fn games(store: &Store, query: GameQuery) -> Result<Vec<Game>, StoreError> {
    store
        .blocking(move |db| {
            let read = db.begin_read().map_err(storage)?;
            let games_table = read.open_table(GAMES).map_err(storage)?;

            let cutoff = query.stale_after.map(|seconds| query.now - seconds);
            let mut rows: Vec<(u64, GameRow)> = Vec::new();
            for entry in games_table.iter().map_err(storage)? {
                let (entry_key, entry_value) = entry.map_err(storage)?;
                let (id, row) = (entry_key.value(), entry_value.value());
                let (black, white, _, _created_at, updated_at, finished_at) = row;
                let open = black.is_none() || white.is_none();
                let finished = finished_at.is_some();
                let mine = match query.filter {
                    GameFilter::Mine(user_id) => {
                        black == Some(key(user_id)) || white == Some(key(user_id))
                    }
                    _ => false,
                };
                let keep = match query.filter {
                    GameFilter::All => !open || cutoff.is_none_or(|cutoff| updated_at >= cutoff),
                    GameFilter::Open => open && cutoff.is_none_or(|cutoff| updated_at >= cutoff),
                    GameFilter::Running => !open && !finished,
                    GameFilter::Finished => finished,
                    GameFilter::Mine(_) => mine,
                };
                if keep {
                    rows.push((id, row));
                }
            }

            match query.sort {
                GameSort::Active => rows.sort_by_key(|(id, row)| (Reverse(row.4), Reverse(*id))),
                GameSort::Newest => rows.sort_by_key(|(id, row)| (Reverse(row.3), Reverse(*id))),
                GameSort::Oldest => rows.sort_by_key(|(id, row)| (row.4, *id)),
            }
            rows.truncate(usize::try_from(query.limit).unwrap_or(usize::MAX));

            let users = read.open_table(USERS).map_err(storage)?;
            let moves = read.open_table(MOVES).map_err(storage)?;
            rows.into_iter()
                .map(|(id, row)| build_game(id, row, &users, &moves))
                .collect()
        })
        .await
}

pub async fn moves(store: &Store, game_id: i64) -> Result<Vec<Move>, StoreError> {
    store
        .blocking(move |db| {
            let read = db.begin_read().map_err(storage)?;
            let table = read.open_table(MOVES).map_err(storage)?;
            let mut moves = Vec::new();
            for entry in table.range(move_keys(key(game_id))).map_err(storage)? {
                let (key, value) = entry.map_err(storage)?;
                let (_, seq) = key.value();
                let (colour, point, _, played_at) = value.value();
                let point = match point {
                    None => None,
                    Some(text) => Some(
                        Point::parse_label(text).ok_or(StoreError::Corrupt("move coordinate"))?,
                    ),
                };
                moves.push(Move {
                    seq,
                    color: colour_of(colour)?,
                    point,
                    played_at,
                });
            }
            Ok(moves)
        })
        .await
}

/// Record a move (or a pass) and advance the game.
///
/// One write transaction: load, validate, insert. `expected_seq` is the move
/// number the client last saw; if anything else happened in the meantime the
/// move is refused rather than silently applied to a board the player never
/// looked at.
pub async fn play(
    store: &Store,
    game_id: i64,
    user_id: i64,
    expected_seq: u32,
    point: Option<Point>,
    now: i64,
) -> Result<PlayOutcome, StoreError> {
    store
        .blocking(move |db| {
            let write = db.begin_write().map_err(storage)?;
            let outcome = {
                let mut games = write.open_table(GAMES).map_err(storage)?;
                let mut moves_table = write.open_table(MOVES).map_err(storage)?;
                let row = {
                    let guard = games
                        .get(key(game_id))
                        .map_err(storage)?
                        .ok_or(StoreError::GameNotFound)?;
                    let (black, white, seq, created_at, updated_at, finished_at) = guard.value();
                    (black, white, seq, created_at, updated_at, finished_at)
                };
                let (black, white, seq, created_at, _updated_at, finished_at) = row;
                if finished_at.is_some() {
                    return Err(StoreError::GameFinished);
                }
                let turn = if seq.is_multiple_of(2) {
                    Cell::Black
                } else {
                    Cell::White
                };
                // A game may be waiting for an opponent and still be playable:
                // whoever holds black may open while the white seat is free.
                // What blocks a move is the seat *to move* being empty.
                let seat_to_move = match turn {
                    Cell::Black => black,
                    _ => white,
                };
                if seat_to_move.is_none() {
                    return Err(StoreError::GameNotStarted);
                }
                let mine = match turn {
                    Cell::Black => black,
                    _ => white,
                };
                if mine != Some(key(user_id)) {
                    let seated = black == Some(key(user_id)) || white == Some(key(user_id));
                    return Err(if seated {
                        StoreError::NotYourTurn
                    } else {
                        StoreError::NotASeat
                    });
                }
                if seq != expected_seq {
                    return Err(StoreError::StaleBoard);
                }

                let current = position_at(&moves_table, key(game_id), seq)?.unwrap_or_default();
                let (next, captured) = match point {
                    // A pass changes nothing on the board and is exempt from superko.
                    None => (current, Vec::new()),
                    Some(point) => {
                        let seen: Positions = moves_table
                            .range(move_keys(key(game_id)))
                            .map_err(storage)?
                            .map(|entry| {
                                let (_, value) = entry.map_err(storage)?;
                                Ok(value.value().2.to_vec())
                            })
                            .collect::<Result<HashSet<_>, StoreError>>()?;
                        crate::go::rules::place(&current, turn, point, &seen)?
                    }
                };

                let previous_was_pass = if seq >= 1 {
                    moves_table
                        .get((key(game_id), seq))
                        .map_err(storage)?
                        .map(|guard| guard.value().1.is_none())
                        .unwrap_or(false)
                } else {
                    false
                };
                let finished = point.is_none() && previous_was_pass;

                let next_seq = seq + 1;
                let label = point.map(|point| point.label());
                moves_table
                    .insert(
                        (key(game_id), next_seq),
                        (turn.lower_name(), label.as_deref(), next.as_bytes(), now),
                    )
                    .map_err(storage)?;

                let finished_at = if finished { Some(now) } else { finished_at };
                games
                    .insert(
                        key(game_id),
                        (black, white, next_seq, created_at, now, finished_at),
                    )
                    .map_err(storage)?;
                PlayOutcome {
                    seq: next_seq,
                    state: state_of(black, white, finished_at),
                    captured,
                }
            };
            write.commit().map_err(storage)?;
            Ok(outcome)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: u8, y: u8) -> Point {
        Point::new(x, y).unwrap()
    }

    fn query(filter: GameFilter, sort: GameSort, stale: Option<i64>) -> GameQuery {
        GameQuery {
            filter,
            sort,
            limit: 20,
            now: 1_001,
            stale_after: stale,
        }
    }

    async fn people() -> (Store, User, User) {
        let store = Store::in_memory().unwrap();
        let alice = create_user(&store, "alice", "hash-a", 1_000).await.unwrap();
        let bob = create_user(&store, "bob", "hash-b", 1_000).await.unwrap();
        (store, alice, bob)
    }

    async fn fixture() -> (Store, User, User, i64) {
        let (store, alice, bob) = people().await;
        let game_id = create_game(&store, alice.id, Cell::Black, Some(&bob), 1_000)
            .await
            .unwrap();
        (store, alice, bob, game_id)
    }

    #[tokio::test]
    async fn usernames_are_unique_ignoring_case() {
        let store = Store::in_memory().unwrap();
        create_user(&store, "Alice", "hash", 1).await.unwrap();
        let error = create_user(&store, "aLiCe", "hash", 1).await.unwrap_err();
        assert!(matches!(error, StoreError::UsernameTaken));
        let record = user_by_username(&store, "ALICE").await.unwrap().unwrap();
        assert_eq!(record.user.username, "Alice");
        // The other tables are untouched by the failed attempt.
        assert!(user_by_id(&store, 2).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn passwords_can_be_replaced() {
        let (store, alice, _) = people().await;
        set_password(&store, alice.id, "new-hash").await.unwrap();
        let record = user_record(&store, alice.id).await.unwrap().unwrap();
        assert_eq!(record.password_hash, "new-hash");
        assert_eq!(record.user.username, "alice");
    }

    #[tokio::test]
    async fn sessions_resolve_to_their_user_and_expire() {
        let store = Store::in_memory().unwrap();
        let alice = create_user(&store, "alice", "hash", 1_000).await.unwrap();
        let token = [7u8; 32];
        create_session(&store, &token, alice.id, 1_000, 100)
            .await
            .unwrap();
        let (found, last_seen) = user_for_session(&store, &token, 1_050)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.username, "alice");
        assert_eq!(last_seen, 1_000);
        assert!(user_for_session(&store, &token, 1_101)
            .await
            .unwrap()
            .is_none());
        delete_session(&store, &token).await.unwrap();
        assert!(user_for_session(&store, &token, 1_050)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn expired_sessions_are_cleaned_up_on_the_next_login() {
        let store = Store::in_memory().unwrap();
        let alice = create_user(&store, "alice", "hash", 1_000).await.unwrap();
        let old = [1u8; 32];
        create_session(&store, &old, alice.id, 1_000, 10)
            .await
            .unwrap();
        let fresh = [2u8; 32];
        create_session(&store, &fresh, alice.id, 2_000, 10)
            .await
            .unwrap();
        assert!(user_for_session(&store, &old, 2_000)
            .await
            .unwrap()
            .is_none());
        assert!(user_for_session(&store, &fresh, 2_000)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn touching_a_session_extends_it() {
        let store = Store::in_memory().unwrap();
        let alice = create_user(&store, "alice", "hash", 1_000).await.unwrap();
        let token = [3u8; 32];
        create_session(&store, &token, alice.id, 1_000, 100)
            .await
            .unwrap();
        touch_session(&store, &token, 1_090, 100).await.unwrap();
        let (_, last_seen) = user_for_session(&store, &token, 1_150)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(last_seen, 1_090);
        // Touching moved the expiry to 1_090 + 100.
        assert!(user_for_session(&store, &token, 1_189)
            .await
            .unwrap()
            .is_some());
        assert!(user_for_session(&store, &token, 1_191)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn logging_out_everywhere_keeps_the_current_session() {
        let store = Store::in_memory().unwrap();
        let alice = create_user(&store, "alice", "hash", 1_000).await.unwrap();
        let here = [1u8; 32];
        let there = [2u8; 32];
        create_session(&store, &here, alice.id, 1_000, 100)
            .await
            .unwrap();
        create_session(&store, &there, alice.id, 1_000, 100)
            .await
            .unwrap();
        delete_sessions_for_user(&store, alice.id, Some(here))
            .await
            .unwrap();
        assert!(user_for_session(&store, &here, 1_050)
            .await
            .unwrap()
            .is_some());
        assert!(user_for_session(&store, &there, 1_050)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn a_game_can_be_opened_without_an_opponent() {
        let (store, alice, _) = people().await;
        let game_id = create_game(&store, alice.id, Cell::Black, None, 1_000)
            .await
            .unwrap();
        let game = self::game(&store, game_id).await.unwrap();
        assert_eq!(game.state, GameState::Waiting);
        assert_eq!(game.open_seat(), Some(Cell::White));
        assert_eq!(game.black.as_deref(), Some("alice"));
        assert_eq!(game.white, None);
        assert_eq!(game.turn(), Cell::Black);
        assert!(game.position.is_none());
    }

    #[tokio::test]
    async fn black_may_open_before_an_opponent_sits_down() {
        let (store, alice, bob) = people().await;
        let game_id = create_game(&store, alice.id, Cell::Black, None, 1_000)
            .await
            .unwrap();

        let outcome = play(&store, game_id, alice.id, 0, Some(p(3, 3)), 1_001)
            .await
            .unwrap();
        assert_eq!(outcome.seq, 1);
        assert_eq!(outcome.state, GameState::Waiting);

        let error = play(&store, game_id, alice.id, 1, Some(p(15, 15)), 1_002)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::GameNotStarted));

        claim_seat(&store, game_id, bob.id, 1_003).await.unwrap();
        let game = self::game(&store, game_id).await.unwrap();
        assert_eq!(game.state, GameState::Active);
        assert_eq!(game.seat_of(bob.id), Some(Cell::White));
        assert_eq!(game.position.unwrap().stone_count(), 1);
        assert_eq!(game.last_move, Some(p(3, 3)));
        play(&store, game_id, bob.id, 1, Some(p(15, 15)), 1_004)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_game_opened_as_white_waits_for_someone_to_play_black() {
        let (store, alice, bob) = people().await;
        let game_id = create_game(&store, alice.id, Cell::White, None, 1_000)
            .await
            .unwrap();
        let game = self::game(&store, game_id).await.unwrap();
        assert_eq!(game.open_seat(), Some(Cell::Black));
        assert_eq!(game.seat_of(alice.id), Some(Cell::White));
        assert_eq!(game.white.as_deref(), Some("alice"));
        assert_eq!(game.black, None);

        let error = play(&store, game_id, alice.id, 0, Some(p(3, 3)), 1_001)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::GameNotStarted));

        claim_seat(&store, game_id, bob.id, 1_002).await.unwrap();
        play(&store, game_id, bob.id, 0, Some(p(3, 3)), 1_003)
            .await
            .unwrap();
        play(&store, game_id, alice.id, 1, Some(p(15, 15)), 1_004)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn inviting_a_named_opponent_while_playing_white_lets_them_open() {
        let (store, alice, bob) = people().await;
        let game_id = create_game(&store, alice.id, Cell::White, Some(&bob), 1_000)
            .await
            .unwrap();
        let game = self::game(&store, game_id).await.unwrap();
        assert_eq!(game.state, GameState::Active);
        assert_eq!(game.black_id, Some(bob.id));
        assert_eq!(game.white_id, Some(alice.id));
        play(&store, game_id, bob.id, 0, Some(p(3, 3)), 1_001)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn claiming_a_seat_starts_the_game_and_only_once() {
        let (store, alice, bob) = people().await;
        let eve = create_user(&store, "eve", "hash", 1_000).await.unwrap();
        let game_id = create_game(&store, alice.id, Cell::Black, None, 1_000)
            .await
            .unwrap();

        assert!(matches!(
            claim_seat(&store, game_id, alice.id, 1_001)
                .await
                .unwrap_err(),
            StoreError::OwnGame
        ));
        claim_seat(&store, game_id, bob.id, 1_001).await.unwrap();
        let game = self::game(&store, game_id).await.unwrap();
        assert_eq!(game.state, GameState::Active);
        assert_eq!(game.white.as_deref(), Some("bob"));
        assert!(matches!(
            claim_seat(&store, game_id, eve.id, 1_002)
                .await
                .unwrap_err(),
            StoreError::SeatAlreadyTaken
        ));
    }

    #[tokio::test]
    async fn turns_alternate_and_only_the_player_to_move_may_play() {
        let (store, alice, bob, game_id) = fixture().await;
        assert!(matches!(
            play(&store, game_id, bob.id, 0, Some(p(3, 3)), 1_001)
                .await
                .unwrap_err(),
            StoreError::NotYourTurn
        ));
        let outcome = play(&store, game_id, alice.id, 0, Some(p(3, 3)), 1_001)
            .await
            .unwrap();
        assert_eq!(outcome.seq, 1);
        assert_eq!(outcome.state, GameState::Active);
        play(&store, game_id, bob.id, 1, Some(p(15, 15)), 1_002)
            .await
            .unwrap();
        let moves = self::moves(&store, game_id).await.unwrap();
        assert_eq!(moves.len(), 2);
        assert_eq!(moves[0].point, Some(p(3, 3)));
        assert_eq!(moves[0].color, Cell::Black);
        assert_eq!(moves[1].color, Cell::White);
    }

    #[tokio::test]
    async fn a_stale_board_is_refused() {
        let (store, alice, bob, game_id) = fixture().await;
        play(&store, game_id, alice.id, 0, Some(p(3, 3)), 1_001)
            .await
            .unwrap();
        play(&store, game_id, bob.id, 1, Some(p(15, 15)), 1_002)
            .await
            .unwrap();
        let error = play(&store, game_id, alice.id, 0, Some(p(4, 4)), 1_003)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::StaleBoard));
        assert_eq!(self::game(&store, game_id).await.unwrap().seq, 2);
    }

    #[tokio::test]
    async fn two_passes_end_the_game_and_nothing_may_happen_afterwards() {
        let (store, alice, bob, game_id) = fixture().await;
        play(&store, game_id, alice.id, 0, Some(p(3, 3)), 1_001)
            .await
            .unwrap();
        play(&store, game_id, bob.id, 1, Some(p(15, 15)), 1_002)
            .await
            .unwrap();
        let outcome = play(&store, game_id, alice.id, 2, None, 1_003)
            .await
            .unwrap();
        assert_eq!(outcome.state, GameState::Active);
        let outcome = play(&store, game_id, bob.id, 3, None, 1_004).await.unwrap();
        assert_eq!(outcome.state, GameState::Finished);

        let game = self::game(&store, game_id).await.unwrap();
        assert_eq!(game.state, GameState::Finished);
        assert_eq!(game.finished_at, Some(1_004));
        // The last stone is still the marker, not the pass.
        assert_eq!(game.last_move, Some(p(15, 15)));

        let error = play(&store, game_id, alice.id, 4, Some(p(4, 4)), 1_005)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::GameFinished));
    }

    #[tokio::test]
    async fn a_single_pass_does_not_end_the_game() {
        let (store, alice, bob, game_id) = fixture().await;
        play(&store, game_id, alice.id, 0, None, 1_001)
            .await
            .unwrap();
        let outcome = play(&store, game_id, bob.id, 1, Some(p(3, 3)), 1_002)
            .await
            .unwrap();
        assert_eq!(outcome.state, GameState::Active);
        assert_eq!(outcome.seq, 2);
    }

    #[tokio::test]
    async fn superko_is_enforced_end_to_end() {
        let (store, alice, bob, game_id) = fixture().await;
        let black_moves = [(1, 2), (2, 1), (3, 2), (10, 10)];
        let white_moves = [(1, 3), (2, 2), (3, 3), (2, 4)];
        let mut seq = 0;
        for (black, white) in black_moves.into_iter().zip(white_moves) {
            seq = play(
                &store,
                game_id,
                alice.id,
                seq,
                Some(p(black.0, black.1)),
                1_001,
            )
            .await
            .unwrap()
            .seq;
            seq = play(
                &store,
                game_id,
                bob.id,
                seq,
                Some(p(white.0, white.1)),
                1_002,
            )
            .await
            .unwrap()
            .seq;
        }
        let outcome = play(&store, game_id, alice.id, seq, Some(p(2, 3)), 1_003)
            .await
            .unwrap();
        assert_eq!(outcome.captured, vec![p(2, 2)]);
        let error = play(&store, game_id, bob.id, outcome.seq, Some(p(2, 2)), 1_004)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::Illegal(RuleViolation::Superko)));
    }

    #[tokio::test]
    async fn suicide_is_refused() {
        let (store, alice, bob, game_id) = fixture().await;
        // Black walls in the corner, then white tries to fill it.
        let mut seq = 0;
        for (user, point) in [
            (alice.id, p(1, 0)),
            (bob.id, p(10, 10)),
            (alice.id, p(0, 1)),
            (bob.id, p(11, 11)),
            (alice.id, p(10, 12)),
        ] {
            seq = play(&store, game_id, user, seq, Some(point), 1_001)
                .await
                .unwrap()
                .seq;
        }
        let error = play(&store, game_id, bob.id, seq, Some(p(0, 0)), 1_002)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::Illegal(RuleViolation::Suicide)));
    }

    #[tokio::test]
    async fn spectators_are_not_players() {
        let (store, _, _, game_id) = fixture().await;
        let eve = create_user(&store, "eve", "hash", 1_000).await.unwrap();
        let error = play(&store, game_id, eve.id, 0, Some(p(3, 3)), 1_001)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::NotASeat));
    }

    #[tokio::test]
    async fn the_lobby_filters_by_state() {
        let (store, alice, bob) = people().await;
        let open = create_game(&store, alice.id, Cell::Black, None, 1_000)
            .await
            .unwrap();
        let running = create_game(&store, alice.id, Cell::Black, Some(&bob), 1_000)
            .await
            .unwrap();
        let finished = create_game(&store, alice.id, Cell::Black, Some(&bob), 1_000)
            .await
            .unwrap();
        play(&store, finished, alice.id, 0, None, 1_000)
            .await
            .unwrap();
        play(&store, finished, bob.id, 1, None, 1_000)
            .await
            .unwrap();

        let ids = |games: Vec<Game>| games.into_iter().map(|game| game.id).collect::<Vec<_>>();
        assert_eq!(
            ids(
                games(&store, query(GameFilter::Open, GameSort::Active, None))
                    .await
                    .unwrap()
            ),
            vec![open]
        );
        assert_eq!(
            ids(
                games(&store, query(GameFilter::Running, GameSort::Active, None))
                    .await
                    .unwrap()
            ),
            vec![running]
        );
        assert_eq!(
            ids(
                games(&store, query(GameFilter::Finished, GameSort::Active, None))
                    .await
                    .unwrap()
            ),
            vec![finished]
        );
        assert_eq!(
            games(&store, query(GameFilter::All, GameSort::Active, None))
                .await
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            games(
                &store,
                query(GameFilter::Mine(bob.id), GameSort::Active, None)
            )
            .await
            .unwrap()
            .len(),
            2
        );
        assert_eq!(
            games(
                &store,
                query(GameFilter::Mine(alice.id), GameSort::Active, None)
            )
            .await
            .unwrap()
            .len(),
            3
        );
    }

    #[tokio::test]
    async fn the_lobby_sorts_and_limits() {
        let (store, alice, bob) = people().await;
        let first = create_game(&store, alice.id, Cell::Black, Some(&bob), 1_000)
            .await
            .unwrap();
        let second = create_game(&store, alice.id, Cell::Black, Some(&bob), 2_000)
            .await
            .unwrap();
        let third = create_game(&store, alice.id, Cell::Black, Some(&bob), 3_000)
            .await
            .unwrap();
        // The oldest game gets a move, so the three sorts disagree.
        play(&store, first, alice.id, 0, Some(p(3, 3)), 4_000)
            .await
            .unwrap();

        async fn sorted(store: &Store, sort: GameSort) -> Vec<i64> {
            let mut q = query(GameFilter::All, sort, None);
            q.limit = 1;
            games(store, q)
                .await
                .unwrap()
                .into_iter()
                .map(|game| game.id)
                .collect()
        }
        // Most recently played first: the old game that just got a move.
        assert_eq!(sorted(&store, GameSort::Active).await, vec![first]);
        assert_eq!(sorted(&store, GameSort::Newest).await, vec![third]);
        assert_eq!(sorted(&store, GameSort::Oldest).await, vec![second]);
    }

    #[tokio::test]
    async fn a_game_waiting_for_an_opponent_is_open_even_after_a_move() {
        let (store, alice, _) = people().await;
        let game_id = create_game(&store, alice.id, Cell::Black, None, 1_000)
            .await
            .unwrap();
        play(&store, game_id, alice.id, 0, Some(p(3, 3)), 1_001)
            .await
            .unwrap();

        let open = games(&store, query(GameFilter::Open, GameSort::Active, None))
            .await
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].seq, 1);
        assert_eq!(open[0].open_seat(), Some(Cell::White));
        assert_eq!(open[0].last_move, Some(p(3, 3)));
        assert_eq!(open[0].position.as_ref().unwrap().stone_count(), 1);
        assert!(
            games(&store, query(GameFilter::Running, GameSort::Active, None))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_creator_can_withdraw_a_game_nobody_joined() {
        let (store, alice, _) = people().await;
        let game_id = create_game(&store, alice.id, Cell::Black, None, 1_000)
            .await
            .unwrap();
        play(&store, game_id, alice.id, 0, Some(p(3, 3)), 1_001)
            .await
            .unwrap();

        withdraw_game(&store, game_id, alice.id).await.unwrap();
        assert!(matches!(
            self::game(&store, game_id).await.unwrap_err(),
            StoreError::GameNotFound
        ));
        assert!(self::moves(&store, game_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn only_the_seated_player_may_withdraw() {
        let (store, alice, bob) = people().await;
        let game_id = create_game(&store, alice.id, Cell::White, None, 1_000)
            .await
            .unwrap();
        assert!(matches!(
            withdraw_game(&store, game_id, bob.id).await.unwrap_err(),
            StoreError::NotASeat
        ));
        withdraw_game(&store, game_id, alice.id).await.unwrap();
    }

    #[tokio::test]
    async fn a_game_with_two_players_cannot_be_withdrawn() {
        let (store, alice, bob) = people().await;
        let game_id = create_game(&store, alice.id, Cell::Black, None, 1_000)
            .await
            .unwrap();
        claim_seat(&store, game_id, bob.id, 1_001).await.unwrap();
        for user in [alice.id, bob.id] {
            assert!(matches!(
                withdraw_game(&store, game_id, user).await.unwrap_err(),
                StoreError::OpponentJoined
            ));
        }
        assert!(self::game(&store, game_id).await.is_ok());
    }

    #[tokio::test]
    async fn stale_open_seats_drop_off_the_lobby_but_are_still_reachable() {
        let (store, alice, _) = people().await;
        let day = 24 * 60 * 60;
        let old = create_game(&store, alice.id, Cell::Black, None, 1_000)
            .await
            .unwrap();
        let fresh = create_game(&store, alice.id, Cell::Black, None, 1_000 + 10 * day)
            .await
            .unwrap();

        let now = 1_000 + 15 * day;
        let listing = |stale: Option<i64>, filter: GameFilter| {
            let mut q = query(filter, GameSort::Active, stale);
            q.now = now;
            q
        };
        let listed = games(&store, listing(Some(14 * day), GameFilter::Open))
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, fresh);
        assert!(self::game(&store, old).await.is_ok());

        let all = games(&store, listing(None, GameFilter::Open))
            .await
            .unwrap();
        assert_eq!(all.len(), 2);

        let mine = games(&store, listing(Some(14 * day), GameFilter::Mine(alice.id)))
            .await
            .unwrap();
        assert_eq!(mine.len(), 2, "your own games are never hidden from you");

        play(&store, old, alice.id, 0, Some(p(3, 3)), now)
            .await
            .unwrap();
        let listed = games(&store, listing(Some(14 * day), GameFilter::Open))
            .await
            .unwrap();
        assert_eq!(listed.len(), 2, "a move counts as activity");
    }
}
