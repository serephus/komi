//! The store: a redb database, its tables, and the transaction helpers.
//!
//! redb is a pure-Rust B-tree key-value store with MVCC: any number of readers
//! run concurrently with the one writer. That suits this application exactly,
//! because every change here is read-check-write inside a single write
//! transaction.
//!
//! What SQL used to give us for free is now explicit, so it is worth naming
//! where each guarantee lives:
//!
//! * unique usernames — the `usernames` table, written in the same transaction
//!   as the user row;
//! * deleting a game takes its moves — [`crate::store::withdraw_game`] removes
//!   the key range;
//! * a game's state — *derived* from its seats and `finished_at`, so it cannot
//!   disagree with them;
//! * a move's colour and the game's seats — enums and `Option`, not strings that
//!   a constraint has to police.

use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableTable, TableDefinition};

use crate::store::StoreError;

/// Bookkeeping: schema version and the id counters.
pub const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
/// `(username, username_lower, password_hash, created_at)`
pub type UserRow = (&'static str, &'static str, &'static str, i64);
/// `(black_id, white_id, seq, created_at, updated_at, finished_at)`
pub type GameRow = (Option<u64>, Option<u64>, u32, i64, i64, Option<i64>);
/// `(colour, point, board, played_at)`
pub type MoveRow = (&'static str, Option<&'static str>, &'static [u8], i64);

/// `id -> (username, username_lower, password_hash, created_at)`
pub const USERS: TableDefinition<u64, UserRow> = TableDefinition::new("users");
/// `username_lower -> id`. The unique index, maintained by hand.
pub const USERNAMES: TableDefinition<&str, u64> = TableDefinition::new("usernames");
/// `token_hash -> (user_id, created_at, last_seen_at, expires_at)`
pub const SESSIONS: TableDefinition<&[u8], (u64, i64, i64, i64)> = TableDefinition::new("sessions");
/// `id -> (black_id, white_id, seq, created_at, updated_at, finished_at)`
pub const GAMES: TableDefinition<u64, GameRow> = TableDefinition::new("games");
/// `(game_id, seq) -> (colour, point, board, played_at)`
pub const MOVES: TableDefinition<(u64, u32), MoveRow> = TableDefinition::new("moves");

/// Bumped when the shape of the tables changes. Pre-release, a mismatch means
/// the file must be recreated rather than migrated.
const SCHEMA_VERSION: u64 = 1;

const VERSION_KEY: &str = "version";
pub const NEXT_USER: &str = "next_user";
pub const NEXT_GAME: &str = "next_game";

/// A handle to the database. Cloning is cheap (an `Arc`), and every operation
/// runs on a blocking thread so the async runtime is never held up by I/O.
#[derive(Clone, Debug)]
pub struct Store {
    db: Arc<Database>,
}

impl Store {
    /// Open (creating if needed) the database file and check its version.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    StoreError::Storage(format!("could not create {}: {error}", parent.display()))
                })?;
            }
        }
        let db = open_with_retry(path)?;
        let store = Store { db: Arc::new(db) };
        store.initialise()?;
        Ok(store)
    }

    /// An empty database in memory. Used by the tests.
    pub fn in_memory() -> Result<Self, StoreError> {
        let db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .map_err(|error| StoreError::Storage(format!("could not open database: {error}")))?;
        let store = Store { db: Arc::new(db) };
        store.initialise()?;
        Ok(store)
    }

    fn initialise(&self) -> Result<(), StoreError> {
        let write = self.db.begin_write().map_err(storage)?;
        {
            // Open every table once: a read on a table redb has never written
            // returns an error, and a fresh database has not written any of them.
            write.open_table(USERS).map_err(storage)?;
            write.open_table(USERNAMES).map_err(storage)?;
            write.open_table(SESSIONS).map_err(storage)?;
            write.open_table(GAMES).map_err(storage)?;
            write.open_table(MOVES).map_err(storage)?;
            let mut meta = write.open_table(META).map_err(storage)?;
            let version = meta
                .get(VERSION_KEY)
                .map_err(storage)?
                .map(|guard| guard.value());
            match version {
                None => {
                    meta.insert(VERSION_KEY, SCHEMA_VERSION).map_err(storage)?;
                    meta.insert(NEXT_USER, 1).map_err(storage)?;
                    meta.insert(NEXT_GAME, 1).map_err(storage)?;
                }
                Some(found) if found == SCHEMA_VERSION => {}
                Some(_found) => {
                    return Err(StoreError::Corrupt("database schema version"));
                }
            }
        }
        write.commit().map_err(storage)?;
        Ok(())
    }

    /// Run `work` on a blocking thread with the database.
    pub(crate) async fn blocking<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        F: FnOnce(&Database) -> Result<T, StoreError> + Send + 'static,
        T: Send + 'static,
    {
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || work(&db))
            .await
            .map_err(|error| StoreError::Storage(format!("store task failed: {error}")))?
    }
}

pub(crate) fn storage<E: std::fmt::Display>(error: E) -> StoreError {
    StoreError::Storage(error.to_string())
}

/// Open the database, waiting briefly if a previous process is still letting
/// go of it. A restart that begins while the old process is shutting down is
/// normal, and should not need a manual retry.
fn open_with_retry(path: &Path) -> Result<Database, StoreError> {
    let mut last = None;
    for attempt in 0..40 {
        match Database::create(path) {
            Ok(db) => return Ok(db),
            Err(redb::DatabaseError::DatabaseAlreadyOpen) => {
                if attempt == 0 {
                    tracing::info!("database is still open elsewhere; waiting for it");
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
                last = Some(redb::DatabaseError::DatabaseAlreadyOpen);
            }
            Err(error) => {
                return Err(StoreError::Storage(format!(
                    "could not open database: {error}"
                )))
            }
        }
    }
    Err(StoreError::Storage(format!(
        "could not open database: {}",
        last.map(|error| error.to_string()).unwrap_or_default()
    )))
}

/// Take the next id from a counter in the meta table. Only called inside a
/// write transaction, which redb allows one of at a time, so this cannot race.
pub(crate) fn next_id(meta: &mut redb::Table<&str, u64>, key: &str) -> Result<u64, StoreError> {
    let next = meta
        .get(key)
        .map_err(storage)?
        .map(|guard| guard.value())
        .ok_or(StoreError::Corrupt("id counter"))?;
    meta.insert(key, next + 1).map_err(storage)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redb::{ReadableDatabase, ReadableTableMetadata};

    #[tokio::test]
    async fn reopening_keeps_the_counters() {
        let dir = std::env::temp_dir().join(format!("komi-redb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.redb");
        let _ = std::fs::remove_file(&path);

        {
            let store = Store::open(&path).unwrap();
            store
                .blocking(|db| {
                    let write = db.begin_write().map_err(storage)?;
                    {
                        let mut meta = write.open_table(META).map_err(storage)?;
                        next_id(&mut meta, NEXT_USER)?;
                    }
                    write.commit().map_err(storage)?;
                    Ok(())
                })
                .await
                .unwrap();
        }
        {
            let store = Store::open(&path).unwrap();
            let next = store
                .blocking(|db| {
                    let read = db.begin_read().map_err(storage)?;
                    let meta = read.open_table(META).map_err(storage)?;
                    Ok(meta.get(NEXT_USER).map_err(storage)?.unwrap().value())
                })
                .await
                .unwrap();
            assert_eq!(next, 2, "the counter survives reopening");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn every_table_exists_from_the_start() {
        // Reads on a table redb has never written return an error, so the store
        // creates them all when it initialises.
        let store = Store::in_memory().unwrap();
        let counts = store
            .blocking(|db| {
                let read = db.begin_read().map_err(storage)?;
                Ok([
                    read.open_table(META)
                        .map_err(storage)?
                        .len()
                        .map_err(storage)?,
                    read.open_table(USERS)
                        .map_err(storage)?
                        .len()
                        .map_err(storage)?,
                    read.open_table(USERNAMES)
                        .map_err(storage)?
                        .len()
                        .map_err(storage)?,
                    read.open_table(SESSIONS)
                        .map_err(storage)?
                        .len()
                        .map_err(storage)?,
                    read.open_table(GAMES)
                        .map_err(storage)?
                        .len()
                        .map_err(storage)?,
                    read.open_table(MOVES)
                        .map_err(storage)?
                        .len()
                        .map_err(storage)?,
                ])
            })
            .await
            .unwrap();
        assert_eq!(counts[0], 3, "version and two counters");
        for count in &counts[1..] {
            assert_eq!(*count, 0);
        }
    }

    #[tokio::test]
    async fn the_schema_version_is_set_once() {
        let store = Store::in_memory().unwrap();
        let version = store
            .blocking(|db| {
                let read = db.begin_read().map_err(storage)?;
                let meta = read.open_table(META).map_err(storage)?;
                Ok(meta.get(VERSION_KEY).map_err(storage)?.unwrap().value())
            })
            .await
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }
}
