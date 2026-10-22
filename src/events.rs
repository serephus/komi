//! Live updates.
//!
//! One broadcast channel carries "something about game N changed". Every open
//! page subscribes to it over server-sent events; a page reloads when an event
//! concerns it. The event deliberately carries no board or move data: a reload
//! is cheap here and cannot go stale, whereas a patch stream would need to know
//! how to apply itself to HTML the server generated.

use tokio::sync::broadcast;

/// How many events can be in flight before slow subscribers start lagging.
const CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Created,
    Claimed,
    Played,
    Withdrawn,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Created => "created",
            EventKind::Claimed => "claimed",
            EventKind::Played => "played",
            EventKind::Withdrawn => "withdrawn",
        }
    }
}

/// What the client receives: `<game id> <kind>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameEvent {
    pub game_id: i64,
    pub kind: EventKind,
}

impl GameEvent {
    pub fn payload(&self) -> String {
        format!("{} {}", self.game_id, self.kind.as_str())
    }
}

/// The publisher side. Cloned into every request.
#[derive(Debug, Clone)]
pub struct Events {
    sender: broadcast::Sender<GameEvent>,
}

impl Default for Events {
    fn default() -> Self {
        Events::new()
    }
}

impl Events {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CAPACITY);
        Events { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GameEvent> {
        self.sender.subscribe()
    }

    /// Nobody listening is not an error: with no pages open there is nobody to
    /// tell, and the database is the record of what happened.
    pub fn publish(&self, game_id: i64, kind: EventKind) {
        let _ = self.sender.send(GameEvent { game_id, kind });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_is_the_game_and_the_kind() {
        let event = GameEvent {
            game_id: 42,
            kind: EventKind::Played,
        };
        assert_eq!(event.payload(), "42 played");
    }

    #[tokio::test]
    async fn subscribers_receive_what_is_published() {
        let events = Events::new();
        let mut receiver = events.subscribe();
        events.publish(7, EventKind::Claimed);
        let received = receiver.recv().await.unwrap();
        assert_eq!(
            received,
            GameEvent {
                game_id: 7,
                kind: EventKind::Claimed
            }
        );
    }

    #[tokio::test]
    async fn publishing_with_nobody_listening_is_harmless() {
        let events = Events::new();
        events.publish(1, EventKind::Created);
    }

    #[tokio::test]
    async fn every_kind_has_a_name() {
        for kind in [
            EventKind::Created,
            EventKind::Claimed,
            EventKind::Played,
            EventKind::Withdrawn,
        ] {
            assert!(!kind.as_str().is_empty());
        }
    }
}
