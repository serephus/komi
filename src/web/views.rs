//! All HTML. Server rendered, no JavaScript required to play, which is only
//! fair for a game about taking your time.
//!
//! Every user-visible string comes from [`crate::i18n`]; the language is passed
//! in explicitly so that a page can be rendered and asserted in any language.

use axum::http::StatusCode;
use maud::{html, Markup, DOCTYPE};

use crate::auth::CurrentUser;
use crate::go::{Board, Cell, Point};
use crate::i18n::{self, fill, t, Lang, Text};
use crate::store::{Game, GameFilter, GameSort, GameState, Move, User};
use crate::util::date_time;
use crate::web::settings;

const MARGIN: f64 = 34.0;
const PITCH: f64 = 20.0;
const GRID: f64 = PITCH * 18.0;
const BOARD_PX: f64 = GRID + 2.0 * MARGIN;

const STAR_POINTS: [(u8, u8); 9] = [
    (3, 3),
    (9, 3),
    (15, 3),
    (3, 9),
    (9, 9),
    (15, 9),
    (3, 15),
    (9, 15),
    (15, 15),
];

fn px(value: f64) -> String {
    format!("{value:.1}")
}

/// Where an intersection sits inside the board, as a percentage of the box.
fn position_percent(point: Point) -> (f64, f64) {
    let left = (MARGIN + point.x as f64 * PITCH) / BOARD_PX * 100.0;
    let top = (MARGIN + (18.0 - point.y as f64) * PITCH) / BOARD_PX * 100.0;
    (left, top)
}

/// The player in a seat, or the translated words for an empty one.
fn player_name(game: &Game, colour: Cell, lang: Lang) -> String {
    game.player(colour)
        .map(str::to_owned)
        .unwrap_or_else(|| t(lang, Text::OpenSeat).to_owned())
}

/// "alice (B) vs bob (W)", with the empty seat spelled out.
fn game_title(game: &Game, lang: Lang) -> String {
    format!(
        "{} (B) {} {} (W)",
        player_name(game, Cell::Black, lang),
        t(lang, Text::Vs),
        player_name(game, Cell::White, lang)
    )
}

/// What the header knows about the visitor.
///
/// `Unknown` is for pages rendered outside a request's handler (error pages),
/// which must not guess: claiming "log in / sign up" to somebody who is already
/// logged in is worse than showing no account links at all.
#[derive(Clone, Copy)]
pub enum Nav<'a> {
    User(&'a User),
    Anonymous,
    Unknown,
}

impl<'a> From<&'a CurrentUser> for Nav<'a> {
    fn from(current: &'a CurrentUser) -> Self {
        match current.user() {
            Some(user) => Nav::User(user),
            None => Nav::Anonymous,
        }
    }
}

fn language_switch(lang: Lang) -> Markup {
    html! {
        nav.langs aria-label=(t(lang, Text::LanguageLabel)) {
            @for option in Lang::ALL {
                a class=(if option == lang { "lang lang-on" } else { "lang" })
                    href=(format!("/lang/{}", option.code()))
                    lang=(option.html())
                    title=(option.label()) {
                    (option.short())
                }
            }
        }
    }
}

fn layout(
    lang: Lang,
    title: &str,
    nav: Nav<'_>,
    allow_registration: bool,
    content: Markup,
) -> Markup {
    html! {
        (DOCTYPE)
        html lang=(lang.html()) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " · " (settings::site_name()) }
                link rel="stylesheet" href=(crate::assets::stylesheet_url());
                script src=(crate::assets::script_url()) defer {}
            }
            body {
                header.site {
                    a.brand href="/" {
                        span.brand-mark { "●" }
                        (settings::site_name())
                    }
                    (language_switch(lang))
                    nav.account {
                        @match nav {
                            Nav::User(user) => {
                                a.who href="/me" { (user.username) }
                                form method="post" action="/logout" {
                                    button.ghost.small type="submit" { (t(lang, Text::LogOut)) }
                                }
                            }
                            Nav::Anonymous => {
                                a href="/login" { (t(lang, Text::LogIn)) }
                                @if allow_registration {
                                    a.button.small href="/register" { (t(lang, Text::SignUp)) }
                                }
                            }
                            Nav::Unknown => {}
                        }
                    }
                }
                main { (content) }
                footer.site { (t(lang, Text::Footer)) }
            }
        }
    }
}

// ------------------------------------------------------------------- board

#[derive(Clone, Copy)]
struct BoardOptions {
    /// Coordinate letters and numbers around the edges.
    coordinates: bool,
    /// A preview: flat stones, no gradients or shadows, no coordinate labels.
    mini: bool,
}

fn board_svg(board: &Board, last: Option<Point>, options: BoardOptions) -> Markup {
    let class = if options.mini {
        "board board-mini"
    } else {
        "board"
    };
    html! {
        svg class=(class)
            viewBox=(format!("0 0 {BOARD_PX} {BOARD_PX}"))
            xmlns="http://www.w3.org/2000/svg"
            role="img"
            aria-label="Go board" {
            @if !options.mini {
                // Only the full board carries these: preview boards are flat,
                // so their ids are never duplicated across a page.
                defs {
                    radialGradient id="stone-black" cx="34%" cy="28%" r="78%" {
                        stop offset="0%" stop-color="#4c515c";
                        stop offset="48%" stop-color="#1c1f26";
                        stop offset="100%" stop-color="#07080a";
                    }
                    radialGradient id="stone-white" cx="34%" cy="28%" r="78%" {
                        stop offset="0%" stop-color="#ffffff";
                        stop offset="55%" stop-color="#f0ebdf";
                        stop offset="100%" stop-color="#c9c1ae";
                    }
                    filter id="stone-shadow" x="-40%" y="-40%" width="180%" height="180%" {
                        feDropShadow dx="0" dy="1" stdDeviation="1" flood-color="#3a2c10" flood-opacity="0.45";
                    }
                    linearGradient id="wood" x1="0" y1="0" x2="0.65" y2="1" {
                        stop offset="0%" stop-color="#e3c073";
                        stop offset="52%" stop-color="#d7b264";
                        stop offset="100%" stop-color="#c8a154";
                    }
                }
            }
            rect.board-bg x="0" y="0" width=(BOARD_PX) height=(BOARD_PX) rx="6" {}
            g.grid {
                @for i in 0..19 {
                    (grid_line(i))
                }
            }
            g.stars {
                @for (x, y) in STAR_POINTS {
                    circle
                        cx=(px(MARGIN + x as f64 * PITCH))
                        cy=(px(MARGIN + (18.0 - y as f64) * PITCH))
                        r=(if options.mini { "5.0" } else { "2.7" })
                        class="star" {}
                }
            }
            @if options.coordinates {
                g.coords {
                    @for i in 0..19 {
                        text x=(px(MARGIN + i as f64 * PITCH)) y=(px(BOARD_PX - MARGIN / 2.6))
                            class="coord" text-anchor="middle" {
                            (crate::go::coord::column_letter(i))
                        }
                        text x=(px(MARGIN / 2.6)) y=(px(MARGIN + (18.0 - i as f64) * PITCH + 3.0))
                            class="coord" text-anchor="middle" {
                            (i + 1)
                        }
                    }
                }
            }
            g.stones {
                @for index in 0..crate::go::POINTS {
                    @let cell = board.get_index(index);
                    @if cell != Cell::Empty {
                        @let point = Point::from_index(index);
                        circle
                            cx=(px(MARGIN + point.x as f64 * PITCH))
                            cy=(px(MARGIN + (18.0 - point.y as f64) * PITCH))
                            r=(if options.mini { "12.8" } else { "9.5" })
                            class=(if cell == Cell::Black { "stone stone-black" } else { "stone stone-white" }) {}
                    }
                }
            }
            @if let Some(point) = last {
                circle
                    cx=(px(MARGIN + point.x as f64 * PITCH))
                    cy=(px(MARGIN + (18.0 - point.y as f64) * PITCH))
                    r=(if options.mini { "6" } else { "4" })
                    class="last-move" {}
            }
            // A crisp edge, so the board does not read as a flat rectangle.
            rect.board-rim x="1" y="1" width=(BOARD_PX - 2.0) height=(BOARD_PX - 2.0) rx="5" {}
        }
    }
}

fn grid_line(i: usize) -> Markup {
    let offset = MARGIN + i as f64 * PITCH;
    let start = MARGIN;
    let end = MARGIN + GRID;
    html! {
        line x1=(px(start)) y1=(px(offset)) x2=(px(end)) y2=(px(offset)) class="grid-line" {}
        line x1=(px(offset)) y1=(px(start)) x2=(px(offset)) y2=(px(end)) class="grid-line" {}
    }
}

/// One submit button per empty intersection, all pointing at the move form.
/// The ghost stone under the cursor is pure CSS on these buttons.
fn point_buttons(game_id: i64, seq: u32, board: &Board) -> Markup {
    html! {
        form id="move-form" method="post" action=(format!("/g/{game_id}/move")) {
            input type="hidden" name="seq" value=(seq);
        }
        @for index in 0..crate::go::POINTS {
            @let point = Point::from_index(index);
            @if board.get_index(index) == Cell::Empty {
                @let (left, top) = position_percent(point);
                button.point
                    type="submit"
                    form="move-form"
                    name="point"
                    value=(point.label())
                    title=(point.label())
                    aria-label=(point.label())
                    style=(format!("left:{left:.3}%;top:{top:.3}%;"));
            }
        }
    }
}

// ------------------------------------------------------------ shared bits

fn status_badge(game: &Game, seat: Option<Cell>, now: i64, lang: Lang) -> Markup {
    let (class, label) = match game.state {
        GameState::Finished => ("badge badge-finished", t(lang, Text::Finished).to_owned()),
        _ => {
            let to_move = game.turn();
            if game.seat(to_move).is_none() {
                (
                    "badge badge-waiting",
                    t(lang, Text::WaitingForOpponent).to_owned(),
                )
            } else if seat == Some(to_move) {
                ("badge badge-you", t(lang, Text::YourMove).to_owned())
            } else {
                (
                    "badge badge-them",
                    fill(
                        t(lang, Text::ToMove),
                        &[("name", &player_name(game, to_move, lang))],
                    ),
                )
            }
        }
    };
    html! {
        span class=(class) { (label) }
        span.quiet { " · " (i18n::ago(lang, now, game.updated_at)) }
    }
}

/// One row in the lobby: a preview of the position, the players, and a way to
/// take the free seat if there is one.
fn game_card(game: &Game, viewer: Option<i64>, now: i64, lang: Lang) -> Markup {
    let seat = viewer.and_then(|user_id| game.seat_of(user_id));
    let claim = match (viewer, game.open_seat()) {
        (Some(_), Some(open)) if seat.is_none() => Some(open),
        _ => None,
    };
    let position = game.position.clone().unwrap_or_default();
    html! {
        li.game {
            a.preview href=(format!("/g/{}", game.id)) tabindex="-1" aria-hidden="true" {
                (board_svg(&position, game.last_move, BoardOptions { coordinates: false, mini: true }))
            }
            div.game-main {
                a.game-title href=(format!("/g/{}", game.id)) {
                    span.side { (player_name(game, Cell::Black, lang)) " (B)" }
                    span.vs { (t(lang, Text::Vs)) }
                    span.side { (player_name(game, Cell::White, lang)) " (W)" }
                }
                p.game-meta {
                    (status_badge(game, seat, now, lang))
                    span.quiet { " · " (fill(t(lang, Text::MoveCount), &[("count", &game.seq.to_string())])) }
                }
            }
            @if let Some(open) = claim {
                form.game-side method="post" action=(format!("/games/{}/claim", game.id)) {
                    button.ghost type="submit" {
                        (fill(t(lang, Text::TakeSeat), &[("colour", &colour_word(open, lang))]))
                    }
                }
            }
        }
    }
}

/// The word for a colour as it appears inside a sentence. Shorter than the
/// caption: "white", 白, and in Korean 백 rather than the caption 흑/백 pair.
fn colour_word(colour: Cell, lang: Lang) -> String {
    match colour {
        Cell::Black => t(lang, Text::ColourBlack),
        _ => t(lang, Text::ColourWhite),
    }
    .to_owned()
}

fn game_list(games: &[Game], viewer: Option<i64>, now: i64, lang: Lang, empty: Text) -> Markup {
    html! {
        @if games.is_empty() {
            p.empty { (t(lang, empty)) }
        } @else {
            ul.games {
                @for game in games {
                    (game_card(game, viewer, now, lang))
                }
            }
        }
    }
}

fn lobby_url(filter: GameFilter, sort: GameSort) -> String {
    format!("/?filter={}&sort={}", filter.key(), sort.key())
}

fn chip(url: &str, label: &str, active: bool) -> Markup {
    html! {
        a class=(if active { "chip chip-on" } else { "chip" }) href=(url) { (label) }
    }
}

fn filter_text(filter: GameFilter) -> Text {
    match filter {
        GameFilter::All => Text::FilterAll,
        GameFilter::Open => Text::FilterOpen,
        GameFilter::Running => Text::FilterRunning,
        GameFilter::Finished => Text::FilterFinished,
        GameFilter::Mine(_) => Text::FilterMine,
    }
}

fn sort_text(sort: GameSort) -> Text {
    match sort {
        GameSort::Active => Text::SortActive,
        GameSort::Newest => Text::SortNewest,
        GameSort::Oldest => Text::SortQuietest,
    }
}

fn empty_text(filter: GameFilter) -> Text {
    match filter {
        GameFilter::All => Text::EmptyAll,
        GameFilter::Open => Text::EmptyOpen,
        GameFilter::Running => Text::EmptyRunning,
        GameFilter::Finished => Text::EmptyFinished,
        GameFilter::Mine(_) => Text::EmptyMine,
    }
}

// ------------------------------------------------------------------ pages

pub fn error_page(lang: Lang, status: StatusCode, title: Text, message: &str) -> Markup {
    let status_title = match status {
        StatusCode::NOT_FOUND => Text::TitleNotFound,
        StatusCode::UNAUTHORIZED => Text::TitleNotLoggedIn,
        StatusCode::FORBIDDEN => Text::TitleRefused,
        StatusCode::TOO_MANY_REQUESTS => Text::TitleSlowDown,
        StatusCode::CONFLICT => Text::TitleConflict,
        _ => Text::TitleUnexpected,
    };
    // The caller may pass a more specific heading; anything else falls back to
    // the one for the status code.
    let heading = if matches!(
        title,
        Text::TitleNotFound
            | Text::TitleNotLoggedIn
            | Text::TitleRefused
            | Text::TitleSlowDown
            | Text::TitleConflict
            | Text::TitleUnexpected
    ) {
        title
    } else {
        status_title
    };
    layout(
        lang,
        t(lang, heading),
        Nav::Unknown,
        false,
        html! {
            section.card.narrow {
                h1 { (t(lang, heading)) }
                p { (message) }
                p.links {
                    a.button href="/" { (t(lang, Text::BackToLobby)) }
                }
            }
        },
    )
}

pub fn game_error_page(
    lang: Lang,
    nav: Nav<'_>,
    allow_registration: bool,
    back: &str,
    message: &str,
) -> Markup {
    layout(
        lang,
        t(lang, Text::MoveRefused),
        nav,
        allow_registration,
        html! {
            section.card.narrow {
                h1 { (t(lang, Text::MoveRefused)) }
                p.error { (message) }
                p.links {
                    a.button href=(back) { (t(lang, Text::BackToBoard)) }
                    a.quiet href="/" { (t(lang, Text::BackToLobby)) }
                }
            }
        },
    )
}

fn create_card(lang: Lang) -> Markup {
    html! {
        section.card.create {
            h2 { (t(lang, Text::NewGame)) }
            form.inline method="post" action="/games" {
                fieldset.colour {
                    legend { (t(lang, Text::PlayAs)) }
                    label.stone-option {
                        input type="radio" name="color" value="black" checked;
                        span.stone-dot.stone-dot-black aria-hidden="true" {}
                        span.stone-caption { (t(lang, Text::Black)) }
                    }
                    label.stone-option {
                        input type="radio" name="color" value="white";
                        span.stone-dot.stone-dot-white aria-hidden="true" {}
                        span.stone-caption { (t(lang, Text::White)) }
                    }
                }
                input
                    id="opponent"
                    name="opponent"
                    type="text"
                    placeholder=(t(lang, Text::OpponentPlaceholder))
                    aria-label=(t(lang, Text::OpponentAria))
                    autocomplete="off";
                button type="submit" { (t(lang, Text::CreateGame)) }
            }
            p.hint { (t(lang, Text::CreateHint)) }
        }
    }
}

pub fn index_page(
    current: &CurrentUser,
    games: &[Game],
    filter: GameFilter,
    sort: GameSort,
    now: i64,
    allow_registration: bool,
    lang: Lang,
) -> Markup {
    let mut filters = vec![
        GameFilter::All,
        GameFilter::Open,
        GameFilter::Running,
        GameFilter::Finished,
    ];
    if let Some(user_id) = current.id() {
        filters.push(GameFilter::Mine(user_id));
    }
    let sorts = [GameSort::Active, GameSort::Newest, GameSort::Oldest];

    layout(
        lang,
        t(lang, Text::LobbyTitle),
        Nav::from(current),
        allow_registration,
        html! {
            section.hero {
                h1 { (t(lang, Text::HeroTitle)) }
                p.lede { (t(lang, Text::HeroLede)) }
            }

            @match current.user() {
                Some(_) => (create_card(lang)),
                None => {
                    section.card.create {
                        h2 { (t(lang, Text::TakeASeat)) }
                        p {
                            (t(lang, Text::TakeASeatIntro)) " "
                            @if allow_registration {
                                a href="/register" { (t(lang, Text::SignUp)) } " "
                                (t(lang, Text::Or)) " "
                            }
                            a href="/login" { (t(lang, Text::LogIn)) } " "
                            (t(lang, Text::TakeASeatTail))
                        }
                    }
                }
            }

            section.browse {
                // The lobby has no game id: any change at all is worth a reload.
                div data-stream="/stream" data-refresh=(settings::refresh_seconds()) {}
                div.toolbar {
                    nav.chips aria-label=(t(lang, Text::SortLabel)) {
                        @for option in &filters {
                            (chip(
                                &lobby_url(*option, sort),
                                t(lang, filter_text(*option)),
                                *option == filter,
                            ))
                        }
                    }
                    nav.chips.chips-sort aria-label=(t(lang, Text::SortLabel)) {
                        span.toolbar-label { (t(lang, Text::SortLabel)) }
                        @for option in sorts {
                            (chip(
                                &lobby_url(filter, option),
                                t(lang, sort_text(option)),
                                option == sort,
                            ))
                        }
                    }
                }
                (game_list(games, current.id(), now, lang, empty_text(filter)))
            }
        },
    )
}

fn move_rows(moves: &[Move]) -> Vec<(u32, Option<String>, Option<String>)> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < moves.len() {
        let number = (index / 2 + 1) as u32;
        let black = moves.get(index).and_then(|m| m.point).map(|p| p.label());
        let white = moves
            .get(index + 1)
            .and_then(|m| m.point)
            .map(|p| p.label());
        rows.push((number, black, white));
        index += 2;
    }
    rows
}

pub fn game_page(
    game: &Game,
    moves: &[Move],
    board: &Board,
    viewer: &CurrentUser,
    now: i64,
    allow_registration: bool,
    lang: Lang,
) -> Markup {
    let seat = viewer.id().and_then(|user_id| game.seat_of(user_id));
    // A game that is still looking for a player is playable for whoever holds
    // the seat to move — in practice black, who may open before white arrives.
    let playable = game.state != GameState::Finished && seat == Some(game.turn());
    let last = moves.iter().rev().find_map(|m| m.point);
    let title = game_title(game, lang);
    let turn_class = if playable {
        match game.turn() {
            Cell::Black => "board-wrap turn-black",
            _ => "board-wrap turn-white",
        }
    } else {
        "board-wrap"
    };

    layout(
        lang,
        &title,
        Nav::from(viewer),
        allow_registration,
        html! {
            section.game-card {
                @if game.state != GameState::Finished {
                    div
                        data-stream="/stream"
                        data-game=(game.id)
                        data-seq=(game.seq)
                        data-fallback=(format!("/g/{}/version", game.id))
                        data-refresh=(settings::refresh_seconds()) {}
                }
                header.game-head {
                    h1 {
                        span.side { (player_name(game, Cell::Black, lang)) " (B)" }
                        span.vs { (t(lang, Text::Vs)) }
                        span.side { (player_name(game, Cell::White, lang)) " (W)" }
                    }
                    p.status {
                        (status_badge(game, seat, now, lang))
                        span.quiet {
                            " · "
                            (fill(t(lang, Text::MoveCount), &[("count", &game.seq.to_string())]))
                        }
                    }
                }

                @if game.state == GameState::Finished {
                    p.banner { (t(lang, Text::NonResultBanner)) }
                }

                div class=(turn_class) {
                    (board_svg(board, last, BoardOptions { coordinates: true, mini: false }))
                    @if playable {
                        (point_buttons(game.id, game.seq, board))
                    }
                }

                div.actions {
                    @if playable {
                        form method="post" action=(format!("/g/{}/pass", game.id)) {
                            input type="hidden" name="seq" value=(game.seq);
                            button.pass type="submit" { (t(lang, Text::Pass)) }
                        }
                        p.hint { (t(lang, Text::PassHint)) }
                    } @else if game.seat(game.turn()).is_some() && game.state != GameState::Finished {
                        p.hint {
                            (fill(
                                t(lang, Text::ItsMove),
                                &[("name", &player_name(game, game.turn(), lang))],
                            ))
                        }
                    }

                    @if let Some(open) = game.open_seat() {
                        @if game.state != GameState::Finished {
                            @match (viewer.id(), seat) {
                                (None, _) => {
                                    p.hint {
                                        (fill(t(lang, Text::SeatFree), &[("colour", &colour_word(open, lang))]))
                                        " "
                                        a href=(format!("/login?next=/g/{}", game.id)) { (t(lang, Text::LogIn)) }
                                        " "
                                        (t(lang, Text::LogInToTake))
                                    }
                                }
                                (Some(_), Some(_)) => {
                                    p.hint {
                                        (fill(t(lang, Text::SeatFree), &[("colour", &colour_word(open, lang))]))
                                        " "
                                        (t(lang, Text::ShareLink))
                                    }
                                }
                                (Some(_), None) => {
                                    p.hint {
                                        (fill(t(lang, Text::SeatFree), &[("colour", &colour_word(open, lang))]))
                                    }
                                    form method="post" action=(format!("/games/{}/claim", game.id)) {
                                        button type="submit" {
                                            (fill(t(lang, Text::TakeSeat), &[("colour", &colour_word(open, lang))]))
                                        }
                                    }
                                }
                            }
                        }
                    }

                    @if game.is_open() && seat.is_some() && game.state != GameState::Finished {
                        form method="post" action=(format!("/games/{}/withdraw", game.id)) {
                            button.ghost type="submit" { (t(lang, Text::WithdrawGame)) }
                        }
                        p.hint { (t(lang, Text::WithdrawHint)) }
                    }
                }

                @if !moves.is_empty() {
                    div.movelist {
                        table {
                            thead {
                                tr {
                                    th { "#" }
                                    th { (t(lang, Text::Black)) }
                                    th { (t(lang, Text::White)) }
                                }
                            }
                            tbody {
                                @for (number, black, white) in move_rows(moves) {
                                    tr {
                                        td.num { (number) }
                                        td {
                                            @match black {
                                                None => { (t(lang, Text::PassedMove)) }
                                                Some(point) => { (point) }
                                            }
                                        }
                                        td {
                                            @match white {
                                                None => {}
                                                Some(point) => { (point) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        @if let Some(last) = moves.last() {
                            p.hint {
                                (fill(t(lang, Text::LastMove), &[("when", &date_time(last.played_at))]))
                            }
                        }
                    }
                }

                p.links {
                    a href=(format!("/g/{}/sgf", game.id)) { (t(lang, Text::DownloadSgf)) }
                }
            }
        },
    )
}

pub fn login_page(
    lang: Lang,
    next: &str,
    error: Option<&str>,
    username: &str,
    allow_registration: bool,
) -> Markup {
    layout(
        lang,
        t(lang, Text::LoginTitle),
        Nav::Anonymous,
        allow_registration,
        html! {
            section.card.narrow {
                h1 { (t(lang, Text::LoginTitle)) }
                @if let Some(error) = error {
                    p.error { (error) }
                }
                form.stacked method="post" action="/login" {
                    input type="hidden" name="next" value=(next);
                    label for="username" { (t(lang, Text::NameLabel)) }
                    input #username name="username" type="text" value=(username) autocomplete="username" required;
                    label for="password" { (t(lang, Text::PasswordLabel)) }
                    input #password name="password" type="password" autocomplete="current-password" required;
                    button type="submit" { (t(lang, Text::LogIn)) }
                }
                @if allow_registration {
                    p.hint {
                        (t(lang, Text::NoAccount)) " "
                        a href="/register" { (t(lang, Text::SignUp)) }
                    }
                }
            }
        },
    )
}

pub fn register_page(lang: Lang, error: Option<&str>, username: &str) -> Markup {
    layout(
        lang,
        t(lang, Text::RegisterTitle),
        Nav::Anonymous,
        true,
        html! {
            section.card.narrow {
                h1 { (t(lang, Text::RegisterTitle)) }
                p.hint { (t(lang, Text::RegisterHint)) }
                @if let Some(error) = error {
                    p.error { (error) }
                }
                form.stacked method="post" action="/register" {
                    label for="username" { (t(lang, Text::NameLabel)) }
                    input #username name="username" type="text" value=(username) autocomplete="username" required;
                    label for="password" { (t(lang, Text::PasswordLabel)) }
                    input #password name="password" type="password" autocomplete="new-password" required;
                    button type="submit" { (t(lang, Text::CreateAccount)) }
                }
                p.hint {
                    (t(lang, Text::AlreadyHaveOne)) " "
                    a href="/login" { (t(lang, Text::LogIn)) }
                }
            }
        },
    )
}

pub fn me_page(
    me: &CurrentUser,
    games: &[Game],
    now: i64,
    session_days: i64,
    message: Option<Text>,
    lang: Lang,
) -> Markup {
    let username = me.user().map(|user| user.username.as_str()).unwrap_or("");
    let expire = if session_days == 1 {
        Text::SessionsExpireOne
    } else {
        Text::SessionsExpire
    };
    layout(
        lang,
        t(lang, Text::AccountTitle),
        Nav::from(me),
        true,
        html! {
            section.hero {
                h1 { (username) }
                @if let Some(message) = message {
                    p.ok { (t(lang, message)) }
                }
                p.hint {
                    (fill(
                        t(lang, expire),
                        &[("count", &session_days.to_string())],
                    ))
                }
            }

            section.browse {
                div data-stream="/stream" data-refresh=(settings::refresh_seconds()) {}
                div.toolbar {
                    h2 { (t(lang, Text::YourGames)) }
                    a.chip href="/?filter=mine&sort=active" { (t(lang, Text::OpenInLobby)) }
                }
                (game_list(games, me.id(), now, lang, Text::EmptyMine))
            }

            section.card.narrow {
                h2 { (t(lang, Text::ChangePassword)) }
                form.stacked method="post" action="/me/password" {
                    label for="current" { (t(lang, Text::CurrentPassword)) }
                    input #current name="current" type="password" autocomplete="current-password" required;
                    label for="password" { (t(lang, Text::NewPassword)) }
                    input #password name="password" type="password" autocomplete="new-password" required;
                    button type="submit" { (t(lang, Text::ChangePasswordSubmit)) }
                }
                p.hint { (t(lang, Text::ChangePasswordHint)) }
            }

            section.card.narrow {
                h2 { (t(lang, Text::ThisDevice)) }
                form.inline method="post" action="/me/logout-all" {
                    button.ghost type="submit" { (t(lang, Text::LogOutEverywhere)) }
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Session;

    fn viewer(id: Option<i64>) -> CurrentUser {
        CurrentUser(id.map(|id| Session {
            user: User {
                id,
                username: match id {
                    1 => "alice".to_owned(),
                    2 => "bob".to_owned(),
                    _ => format!("user{id}"),
                },
            },
            token_hash: [0u8; 32],
        }))
    }

    fn game(seq: u32, state: GameState) -> Game {
        Game {
            id: 7,
            black_id: Some(1),
            white_id: Some(2),
            black: Some("alice".to_owned()),
            white: Some("bob".to_owned()),
            state,
            seq,
            created_at: 1_000,
            updated_at: 1_000,
            finished_at: None,
            position: None,
            last_move: None,
        }
    }

    /// A game somebody opened as black without naming an opponent.
    fn open_seat_game(seq: u32) -> Game {
        let mut game = game(seq, GameState::Waiting);
        game.white_id = None;
        game.white = None;
        game
    }

    fn lobby(current: &CurrentUser, games: &[Game], lang: Lang) -> String {
        index_page(
            current,
            games,
            GameFilter::All,
            GameSort::Active,
            1_400,
            true,
            lang,
        )
        .into_string()
    }

    fn page(game: &Game, viewer: &CurrentUser, lang: Lang) -> String {
        game_page(game, &[], &Board::new(), viewer, 1_400, true, lang).into_string()
    }

    #[test]
    fn move_rows_pair_black_and_white() {
        let moves = vec![
            Move {
                seq: 1,
                color: Cell::Black,
                point: Point::new(3, 15),
                played_at: 0,
            },
            Move {
                seq: 2,
                color: Cell::White,
                point: None,
                played_at: 0,
            },
            Move {
                seq: 3,
                color: Cell::Black,
                point: Point::new(15, 3),
                played_at: 0,
            },
        ];
        let rows = move_rows(&moves);
        assert_eq!(
            rows,
            vec![
                (1, Some("D16".to_owned()), None),
                (2, Some("Q4".to_owned()), None),
            ]
        );
    }

    #[test]
    fn the_board_draws_grid_stars_and_stones() {
        let markup = board_svg(
            &Board::new(),
            None,
            BoardOptions {
                coordinates: true,
                mini: false,
            },
        )
        .into_string();
        assert_eq!(markup.matches("class=\"grid-line\"").count(), 38);
        assert_eq!(markup.matches("class=\"star\"").count(), 9);
        assert!(markup.contains("class=\"coord\""));
        assert!(markup.contains("<defs>"));

        let mut board = Board::new();
        board
            .try_play(Cell::Black, Point::new(3, 3).unwrap())
            .unwrap();
        let markup = board_svg(
            &board,
            Some(Point::new(3, 3).unwrap()),
            BoardOptions {
                coordinates: true,
                mini: false,
            },
        )
        .into_string();
        assert!(markup.contains("stone stone-black"));
        assert!(markup.contains("last-move"));
    }

    #[test]
    fn a_preview_board_is_flat_and_unlabelled() {
        let markup = board_svg(
            &Board::new(),
            None,
            BoardOptions {
                coordinates: false,
                mini: true,
            },
        )
        .into_string();
        assert!(markup.contains("board board-mini"));
        assert!(!markup.contains("class=\"coord\""));
        assert!(!markup.contains("<defs>"), "no duplicate gradient ids");
    }

    #[test]
    fn the_header_shows_the_logged_in_user_on_a_game_page() {
        // Regression: the game page used to render a header that knew nothing
        // about the viewer, so it always claimed you were logged out.
        let markup = page(&game(0, GameState::Active), &viewer(Some(1)), Lang::En);
        assert!(markup.contains("href=\"/me\""));
        assert!(markup.contains(">alice<"));
        assert!(!markup.contains(">Log in<"));
        assert!(!markup.contains(">Sign up<"));
    }

    #[test]
    fn the_header_offers_login_only_to_anonymous_visitors() {
        let markup = page(&game(0, GameState::Active), &viewer(None), Lang::En);
        assert!(markup.contains(">Log in<"));
        assert!(!markup.contains("href=\"/me\""));
    }

    #[test]
    fn error_pages_do_not_guess_who_is_asking() {
        let markup =
            error_page(Lang::En, StatusCode::NOT_FOUND, Text::TitleNotFound, "nope").into_string();
        assert!(markup.contains("nope"));
        assert!(markup.contains("Not found"));
        assert!(!markup.contains(">Log in<"));
        assert!(!markup.contains("href=\"/me\""));
    }

    #[test]
    fn a_finished_game_is_announced_as_a_non_result() {
        let markup = page(&game(2, GameState::Finished), &viewer(Some(1)), Lang::En);
        assert!(markup.contains("no winner, as designed"));
        assert!(!markup.contains("form=\"move-form\""));
        assert!(!markup.contains("data-stream"));
        assert!(markup.contains("Both players passed"));
    }

    #[test]
    fn a_game_in_progress_offers_moves_to_the_player_to_move() {
        let markup = page(&game(0, GameState::Active), &viewer(Some(1)), Lang::En);
        assert!(markup.contains("name=\"point\" value=\"A1\""));
        // The live stream is the primary path; the version endpoint is the fallback.
        assert!(markup.contains("data-stream=\"/stream\""));
        assert!(markup.contains("data-game=\"7\""));
        assert!(markup.contains("data-fallback=\"/g/7/version\""));
        assert!(markup.contains(">Pass<"));
        assert!(markup.contains("board-wrap turn-black"));
    }

    #[test]
    fn the_ghost_stone_matches_the_colour_being_played() {
        let mut white_to_move = game(1, GameState::Active);
        white_to_move.black_id = Some(1);
        white_to_move.white_id = Some(2);
        let markup = game_page(
            &white_to_move,
            &[],
            &Board::new(),
            &viewer(Some(2)),
            1_400,
            true,
            Lang::En,
        )
        .into_string();
        assert!(markup.contains("board-wrap turn-white"));
    }

    #[test]
    fn black_may_move_in_a_game_that_still_needs_an_opponent() {
        let markup = page(&open_seat_game(0), &viewer(Some(1)), Lang::En);
        assert!(markup.contains("name=\"point\" value=\"A1\""));
        assert!(markup.contains("The white seat is free."));
        assert!(markup.contains("your move"));
        assert!(markup.contains("Share this page"));
        assert!(!markup.contains("Take white"));
    }

    #[test]
    fn a_spectator_gets_no_move_buttons() {
        let markup = page(&game(0, GameState::Active), &viewer(None), Lang::En);
        assert!(!markup.contains("name=\"point\""));
        assert!(markup.contains("alice"));
    }

    #[test]
    fn an_open_white_seat_invites_the_right_people() {
        let waiting = open_seat_game(0);
        let anon = page(&waiting, &viewer(None), Lang::En);
        assert!(anon.contains("href=\"/login?next=/g/7\""));
        assert!(!anon.contains("Take white"));

        let visitor = page(&waiting, &viewer(Some(2)), Lang::En);
        assert!(visitor.contains("Take white"));
        // The Korean verb for taking a seat must not be 잡다 (to capture).
        let ko = page(&waiting, &viewer(Some(2)), Lang::Ko);
        assert!(ko.contains("백으로 두기"), "got {ko}");
        let zh = page(&waiting, &viewer(Some(2)), Lang::Zh);
        assert!(zh.contains("执白"));
    }

    #[test]
    fn an_open_black_seat_invites_somebody_to_open_the_game() {
        let mut waiting = game(0, GameState::Waiting);
        waiting.white_id = Some(1);
        waiting.black_id = None;
        waiting.black = None;
        waiting.white = Some("alice".to_owned());

        let visitor = page(&waiting, &viewer(Some(2)), Lang::En);
        assert!(visitor.contains("The black seat is free."));
        assert!(visitor.contains("Take black"));
        assert!(visitor.contains("waiting for an opponent"));

        let owner = page(&waiting, &viewer(Some(1)), Lang::En);
        assert!(!owner.contains("name=\"point\""));
        assert!(owner.contains("Share this page"));
    }

    #[test]
    fn the_owner_of_an_open_game_can_withdraw_it() {
        let markup = page(&open_seat_game(1), &viewer(Some(1)), Lang::En);
        assert!(markup.contains("action=\"/games/7/withdraw\""));
        assert!(markup.contains("Withdraw this game"));
    }

    #[test]
    fn a_game_with_two_players_offers_no_way_out() {
        let markup = page(&game(0, GameState::Active), &viewer(Some(1)), Lang::En);
        assert!(!markup.contains("withdraw"));
    }

    #[test]
    fn the_lobby_offers_a_claim_button_to_the_right_people() {
        let open = vec![open_seat_game(0)];
        assert!(lobby(&viewer(Some(2)), &open, Lang::En).contains("action=\"/games/7/claim\""));
        assert!(lobby(&viewer(Some(2)), &open, Lang::En).contains("Take white"));
        assert!(!lobby(&viewer(Some(1)), &open, Lang::En).contains("action=\"/games/7/claim\""));
        assert!(!lobby(&viewer(None), &open, Lang::En).contains("action=\"/games/7/claim\""));
    }

    #[test]
    fn the_create_form_is_on_top_and_lets_you_pick_a_colour() {
        let markup = lobby(&viewer(Some(1)), &[], Lang::En);
        assert!(markup.contains("name=\"color\" value=\"black\" checked"));
        assert!(markup.contains("name=\"color\" value=\"white\""));
        assert!(markup.contains("placeholder=\"opponent (optional)\""));
        assert_eq!(markup.matches("class=\"stone-option\"").count(), 2);
        let form = markup.find("action=\"/games\"").unwrap();
        let list = markup.find("class=\"browse\"").unwrap();
        assert!(form < list, "New game sits above the list");
    }

    #[test]
    fn anonymous_visitors_get_a_sign_up_card_instead_of_the_form() {
        let markup = lobby(&viewer(None), &[], Lang::En);
        assert!(!markup.contains("action=\"/games\""));
        assert!(markup.contains("href=\"/register\""));
    }

    #[test]
    fn filters_and_sorts_are_links_that_keep_each_other() {
        let markup = index_page(
            &viewer(Some(1)),
            &[],
            GameFilter::Open,
            GameSort::Oldest,
            1_400,
            true,
            Lang::En,
        )
        .into_string();
        // maud escapes the ampersand in attribute values, as it should.
        assert!(
            markup.contains("class=\"chip chip-on\" href=\"/?filter=open&amp;sort=oldest\""),
            "the active filter is marked"
        );
        assert!(markup.contains("href=\"/?filter=all&amp;sort=oldest\""));
        assert!(markup.contains("href=\"/?filter=open&amp;sort=newest\""));
        assert!(markup.contains("href=\"/?filter=mine&amp;sort=oldest\""));
    }

    #[test]
    fn anonymous_visitors_get_no_mine_filter() {
        assert!(!lobby(&viewer(None), &[], Lang::En).contains("filter=mine"));
    }

    #[test]
    fn lobby_rows_carry_a_board_preview() {
        let mut played = open_seat_game(0);
        played.position = Some(Board::new());
        played.last_move = Some(Point::new(3, 3).unwrap());
        let markup = lobby(&viewer(Some(2)), &[played], Lang::En);
        assert!(markup.contains("class=\"board board-mini\""));
        assert!(markup.contains("class=\"preview\""));
        assert!(markup.contains("alice (B)"));
        assert!(markup.contains(">open seat (W)<"));
        assert!(markup.contains("move 0"));
    }

    #[test]
    fn the_empty_state_describes_the_filter() {
        let markup = index_page(
            &viewer(Some(1)),
            &[],
            GameFilter::Finished,
            GameSort::Active,
            1_400,
            true,
            Lang::En,
        )
        .into_string();
        assert!(markup.contains("No games have finished yet."));
    }

    // ------------------------------------------------------------ languages

    #[test]
    fn every_page_renders_in_every_language() {
        let open = vec![open_seat_game(0), game(3, GameState::Active)];
        for lang in Lang::ALL {
            let lobby = lobby(&viewer(Some(1)), &open, lang);
            let game = page(&game(0, GameState::Active), &viewer(Some(1)), lang);
            let account = me_page(&viewer(Some(1)), &open, 1_400, 30, None, lang).into_string();
            let login = login_page(lang, "/", Some("x"), "alice", true).into_string();
            let register = register_page(lang, None, "alice").into_string();
            let error =
                error_page(lang, StatusCode::NOT_FOUND, Text::TitleNotFound, "nope").into_string();
            for page in [lobby, game, account, login, register, error] {
                assert!(
                    page.contains(&format!("<html lang=\"{}\"", lang.html())),
                    "the html lang attribute is set for {lang}"
                );
                // Nothing should have been left in English by accident.
                for english in ["Log in", "Sign up", "Create game", "Your games"] {
                    if lang != Lang::En {
                        assert!(
                            !page.contains(english),
                            "{english:?} is still English in a {lang} page"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_language_switcher_lists_all_four_and_marks_the_current_one() {
        let markup = lobby(&viewer(None), &[], Lang::Ja);
        for code in ["en", "zh", "ja", "ko"] {
            assert!(markup.contains(&format!("href=\"/lang/{code}\"")));
        }
        assert!(markup.contains("class=\"lang lang-on\" href=\"/lang/ja\""));
        assert!(markup.contains("中文"));
        assert!(markup.contains("한국어"));
    }

    #[test]
    fn the_players_and_empty_seats_are_translated() {
        let waiting = open_seat_game(0);
        let games = std::slice::from_ref(&waiting);
        assert!(lobby(&viewer(None), games, Lang::Zh).contains("空位 (W)"));
        assert!(lobby(&viewer(None), games, Lang::Ja).contains("空席 (W)"));
        assert!(lobby(&viewer(None), games, Lang::Ko).contains("빈 자리 (W)"));
    }

    #[test]
    fn status_badges_are_translated() {
        let markup = page(&game(0, GameState::Active), &viewer(Some(1)), Lang::Zh);
        assert!(markup.contains("轮到你"), "got {markup}");
        let markup = page(&game(0, GameState::Finished), &viewer(Some(1)), Lang::Ko);
        assert!(markup.contains("종료"));
    }

    #[test]
    fn the_finish_banner_is_translated() {
        let markup = page(&game(2, GameState::Finished), &viewer(Some(1)), Lang::Ja);
        assert!(markup.contains("勝敗はありません"));
    }

    #[test]
    fn empty_seats_in_sentences_take_the_lowercase_english_word() {
        let markup = page(&open_seat_game(0), &viewer(Some(1)), Lang::En);
        assert!(markup.contains("The white seat is free."));
    }
}
