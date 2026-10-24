# komi

A correspondence Go server with **no clocks and no result**.

Players take as long as they want over a move — days, if that is how it goes.
There is no scoring, no komi, no resignation and no timeout. The only way a game
ends is that both players pass; then it is simply over, with no winner.

19×19, capture, no suicide, and positional superko ("no repeated board").

## Running it

```sh
cargo run
# then open http://127.0.0.1:8080
```

With a config file:

```sh
cp config.example.toml komi.toml   # or pass a path
komi --config /etc/komi/config.toml
```

Or with Nix:

```sh
nix build
./result/bin/komi
```

The first visit asks you to sign up: a name and a password, no email. To start
a game, pick a colour and, if you like, an opponent — leaving the name empty
opens the other seat to anybody, and whoever plays black may make the first move
before that seat is taken. Games are public and watchable; only playing requires
an account.

## Configuration

`komi` reads settings from an optional TOML file and a few environment
variables. In order of increasing precedence:

1. built-in defaults,
2. the config file,
3. the `KOMI_*` environment variables.

The file is looked up at, in order: `--config PATH`, `$KOMI_CONFIG`,
`/etc/komi/config.toml`, `./komi.toml`. A file named explicitly that cannot be
read is an error rather than a silent fall back to defaults, and an unknown key
is reported with its line number — typos are not quietly ignored.

See [`config.example.toml`](config.example.toml) for every key with its default:

```toml
bind = "127.0.0.1:8080"
database = "komi.redb"
site_name = "komi"
log = "info"
secure_cookies = true       # omitted: on unless bind is a loopback address
trust_proxy = false
allow_registration = true
default_language = "en"     # en, zh, ja or ko
refresh_seconds = 30      # fallback only; the live stream does the work

[session]
days = 30

[lobby]
stale_open_days = 14    # free seats stop being listed after this long; 0 = never

[limits]
login_attempts = 10
login_window_minutes = 15
registration_attempts = 5
registration_window_minutes = 60
```

| variable | meaning |
| --- | --- |
| `KOMI_CONFIG` | path to the config file |
| `KOMI_BIND` | overrides `bind` |
| `KOMI_DB` | overrides `database` |
| `KOMI_SECURE_COOKIES` | overrides `secure_cookies` |
| `KOMI_TRUST_PROXY` | overrides `trust_proxy` |
| `KOMI_SESSION_DAYS` | overrides `session.days` |
| `RUST_LOG` | overrides `log` |

The environment only covers what containers and service managers usually need;
everything else lives in the file. There is no secret key to manage: session
tokens are random and only their SHA-256 is stored.

If `database` points into a directory that does not exist, it is created.

## NixOS

The flake exports `nixosModules.default`:

```nix
{
  inputs.komi.url = "github:serephus/komi";

  # in a NixOS configuration:
  imports = [ inputs.komi.nixosModules.default ];

  services.komi = {
    enable = true;
    address = "127.0.0.1";      # keep it local and put a reverse proxy in front
    port = 8080;
    siteName = "my go club";
    secureCookies = true;       # HTTPS terminates at the proxy
    trustProxy = true;          # proxy sets X-Forwarded-For
    allowRegistration = false;  # private instance, accounts already exist
  };
}
```

The module writes komi's own TOML file and passes it with `--config`, so what
the service runs with is inspectable:

```sh
systemctl cat komi
cat $(systemctl show -p ExecStart --value komi | sed 's/.*--config //')
```

Options mirror the config keys: `address`, `port`, `siteName`, `stateDir`,
`user`, `group`, `secureCookies`, `trustProxy`, `allowRegistration`,
`refreshSeconds`, `sessionDays`, `staleOpenDays`, `log`, `limits.*`, plus
`openFirewall`, `package` and `settings` — the last being raw TOML merged last
for anything without an option.

The database lives at `stateDir` (`/var/lib/komi` by default), owned by a
dedicated `komi` user. For the default path systemd itself creates the directory
and takes ownership of it on every start (`StateDirectory`), which is what keeps
a directory left behind by an earlier deployment from locking the service out;
a `chown -R` runs before the service starts as a second line of defence. The unit
runs with `ProtectSystem=strict`, an empty capability set (plus
`CAP_NET_BIND_SERVICE` when the port is below 1024), and only the state directory
writable. Backing up a game history is copying
one redb file, taken while the service is stopped: redb holds a lock on it while
open, and a second process cannot open it either, which is also why the server
waits briefly at startup if a restart overtakes the old process.

## How it is put together

```
src/go/       the rules: board, groups and liberties, coordinates, SGF
src/store.rs  every read and write, and the transaction that records a move
src/db.rs     the redb store: tables, schema version, id counters
src/auth.rs   Argon2id passwords, session cookies, rate limiting, extractors
src/config.rs defaults, TOML file, environment, validation
src/cli.rs    --config, --help, --version
src/web/      routes, handlers, and the server-rendered HTML
src/assets.rs CSS and JS embedded in the binary
nix/module.nix  the NixOS module
```

A few decisions worth knowing about, since they are the parts a "normal" Go
server would do differently:

**The storage is redb, not SQL.** It is an embedded B-tree key-value store in
pure Rust: MVCC, so any number of readers run alongside the one writer, and ACID,
so a move either happens completely or not at all. There is no SQL string to get
wrong and no C toolchain to build; the cost is that the database no longer
enforces anything for us. Unique usernames, deleting a game with its moves, and
a game's state all live in the write transaction and in `src/store.rs`, and a
game's state is *derived* from its seats and its ending so it cannot disagree
with them.

**The move transaction.** A move is validated and recorded in one write
transaction: load the game, check whose turn it is, replay the position, apply
the rules, insert, maybe finish the game. redb allows a single writer at a time,
so there is no window in which two players can both move — a double submit gets
`303` for the first and a stale-board refusal for the second.

**Superko is stored, not hashed.** Every position a game has had is kept
verbatim (361 bytes per move) and compared exactly against the game's own
history. No hash collisions to reason about, and a ko fight is answered by
replaying the game rather than by a special case for ko.

**Passes are exempt from superko.** A pass does not change the board, so a
literal reading of "no repeated board position" would make every pass illegal and
the two-pass ending unreachable. The rule is therefore applied to stone
placements only.

**A game waiting for an opponent is still playable.** Opening a game as black
without naming an opponent does not mean sitting idle: black may play the first
move straight away, and the game stays in the lobby's "open seats" until somebody
takes white and answers. What blocks a move is the seat *to move* being empty,
not the game being incomplete — which is why `black_id` is nullable.

**An open seat is an advertisement, so it expires.** After `lobby.stale_open_days`
without activity a game with a free seat drops off the lobby list; its link keeps
working. Its owner can also withdraw it outright, which deletes the game and its
moves. Once an opponent sits down neither is possible: the game is theirs too, and
there is no result to declare.

**Stale boards are refused, not merged.** Every move form carries the move
number the browser last saw. If the game moved on, the move is refused with a
message instead of being applied to a position the player never looked at.

**Assets are content addressed.** The stylesheet and script are embedded in the
binary and served at a fingerprinted URL (`/static/app.css?v=7223b299`), so a
changed file is a changed URL. That is what stops a browser from running the
previous version of the CSS against new markup.

**No JavaScript is required to play.** The board is an inline SVG, and each empty
intersection is a submit button carrying its own coordinate, all pointing at one
form; the ghost stone that follows the cursor is a CSS pseudo-element on those
buttons, tinted by the colour you would place. The lobby is one filtered list
(`?filter=all|open|running|finished|mine`, `?sort=active|newest|oldest`) built
from links, so the filters work without scripting and the back button behaves;
each row carries a small preview of the position, drawn from the same board
code at a smaller size.

**Live updates are one event stream.** Pages open a server-sent event stream
(`/stream`); a change publishes `<game id> <kind>` and the page reloads itself.
The event carries no game data deliberately: the server already renders the
page, so reloading cannot disagree with the database, and nothing has to keep a
client-side patch correct. On reconnect the page reloads too, since it may have
missed events in between; if the server drops a slow subscriber it sends
`resync`, which also reloads. `refresh_seconds` is only the fallback interval for
a browser without `EventSource`, and `/g/{id}/version` the endpoint it uses.
All of this is an enhancement: without JavaScript, a move is still an ordinary
form post and the page is still correct.

**Sessions are server side.** Cookies hold a random token; the database holds its
hash. Changing a password invalidates every other device.

## Languages

The interface is in English, Chinese (Simplified), Japanese and Korean. Every
string lives in `src/i18n.rs`, one row per string with all four translations
side by side, so a missing translation is a compile error and a wrong one sits
next to its siblings. Strings that need a value are templates with `{name}`
placeholders, and a test checks that no translation loses a placeholder.

The language is chosen per request: the `komi_lang` cookie if the visitor picked
one in the header, otherwise `Accept-Language`, otherwise `default_language`.
Switching is a plain link (`/lang/ja`) that sets the cookie and sends you back to
the page you were on, so it works without JavaScript and the back button behaves.
Traditional Chinese requests are served Simplified — closer than English, and a
better starting point than nothing.

Adding a language means one column in each row of `src/i18n.rs`, a variant in
`Lang`, and a font name in the `font-family` list in `assets/app.css`.

## Tests

```sh
cargo test          # 132 unit and integration tests
nix flake check     # includes evaluating the NixOS module
```

The rules engine has the bulk of the tests — captures, suicide, snapback shapes,
ko, superko, pass handling and SGF round trips — plus end-to-end tests of the
move transaction against an in-memory database (withdrawals, stale seats, the
seat lifecycle), tests for every configuration source and its precedence, and
rendering tests that assert things like "a finished game offers no move
buttons" and "the header shows the logged-in player".

## Not included, on purpose

Ratings, matchmaking, chat, variations, undo, AI opponents, notifications by
email. SGF export is the only export.
