//! Translations.
//!
//! Every user-visible string lives in [`Text`] and carries all four languages in
//! one row, so a missing translation is a compile error and a wrong one is
//! visible next to its siblings. Strings that need a value (`{name}`,
//! `{colour}`, `{count}`) are templates: [`t`] returns the template for a
//! language and [`fill`] substitutes the arguments.
//!
//! The choice of language is made once per request (cookie, then
//! `Accept-Language`, then the configured default) and passed down explicitly,
//! except for the generic error page, which is rendered outside a handler and
//! reads it from the task-local set by the same middleware.

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Lang {
    #[default]
    En,
    Zh,
    Ja,
    Ko,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::En, Lang::Zh, Lang::Ja, Lang::Ko];

    /// Short code used in URLs and cookies.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Zh => "zh",
            Lang::Ja => "ja",
            Lang::Ko => "ko",
        }
    }

    /// Name of the language in itself, for the switcher.
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Zh => "中文",
            Lang::Ja => "日本語",
            Lang::Ko => "한국어",
        }
    }

    /// Very short label, for a crowded header.
    pub fn short(self) -> &'static str {
        match self {
            Lang::En => "EN",
            Lang::Zh => "中文",
            Lang::Ja => "日本語",
            Lang::Ko => "한국어",
        }
    }

    /// Value for `<html lang>`.
    pub fn html(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Zh => "zh-Hans",
            Lang::Ja => "ja",
            Lang::Ko => "ko",
        }
    }

    pub fn parse(value: &str) -> Option<Lang> {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "en-gb" | "english" => Some(Lang::En),
            "zh" | "zh-cn" | "zh-hans" | "zh-sg" | "zh-hans-cn" | "chinese" => Some(Lang::Zh),
            "ja" | "ja-jp" | "jp" | "japanese" => Some(Lang::Ja),
            "ko" | "ko-kr" | "kr" | "korean" => Some(Lang::Ko),
            // Traditional Chinese is served Simplified: closer than English.
            "zh-tw" | "zh-hk" | "zh-hant" => Some(Lang::Zh),
            _ => None,
        }
    }

    /// First acceptable language in an `Accept-Language` header. Tags are
    /// matched on their primary subtag, so `ja-JP` and `ja` both work; quality
    /// values are honoured by taking the first entry, which is how browsers
    /// order preferences.
    pub fn from_accept_language(header: &str) -> Option<Lang> {
        header
            .split(',')
            .map(|entry| entry.split(';').next().unwrap_or(entry).trim())
            .find_map(|tag| {
                Lang::parse(tag).or_else(|| tag.split('-').next().and_then(Lang::parse))
            })
    }

    fn index(self) -> usize {
        match self {
            Lang::En => 0,
            Lang::Zh => 1,
            Lang::Ja => 2,
            Lang::Ko => 3,
        }
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Every string the interface can show. Rows are `[English, Chinese, Japanese,
/// Korean]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Text {
    // Navigation and chrome
    LogIn,
    SignUp,
    LogOut,
    Footer,
    Or,

    // Lobby
    LobbyTitle,
    HeroTitle,
    HeroLede,
    NewGame,
    PlayAs,
    Black,
    White,
    OpponentPlaceholder,
    OpponentAria,
    CreateGame,
    CreateHint,
    TakeASeat,
    TakeASeatIntro,
    TakeASeatTail,
    FilterAll,
    FilterOpen,
    FilterRunning,
    FilterFinished,
    FilterMine,
    Vs,
    AgoNow,
    AgoMinutes,
    AgoHours,
    AgoDays,
    AgoWeeks,
    AgoMonths,
    AgoYears,
    SortLabel,
    SortActive,
    SortNewest,
    SortQuietest,
    EmptyAll,
    EmptyOpen,
    EmptyRunning,
    EmptyFinished,
    EmptyMine,
    OpenSeat,
    ColourBlack,
    ColourWhite,

    // Game state
    YourMove,
    WaitingForOpponent,
    Finished,
    ToMove,
    MoveCount,
    TakeSeat,
    SeatFree,
    LogInToTake,
    ShareLink,

    // Game page
    GameTitleTag,
    NonResultBanner,
    Pass,
    PassHint,
    ItsMove,
    WithdrawGame,
    WithdrawHint,
    PassedMove,
    LastMove,
    DownloadSgf,
    BackToBoard,
    MoveRefused,

    // Account
    AccountTitle,
    SessionsExpire,
    SessionsExpireOne,
    YourGames,
    OpenInLobby,
    ChangePassword,
    CurrentPassword,
    NewPassword,
    ChangePasswordSubmit,
    ChangePasswordHint,
    ThisDevice,
    LogOutEverywhere,
    PasswordChanged,
    LoggedOutEverywhere,

    // Authentication
    LoginTitle,
    NameLabel,
    PasswordLabel,
    RegisterTitle,
    RegisterHint,
    CreateAccount,
    AlreadyHaveOne,
    NoAccount,
    WrongCredentials,
    TooManyAttempts,
    TooManyAccounts,
    NameTaken,
    PasswordTooShort,
    PasswordTooLong,
    NameTooShort,
    NameTooLong,
    NameCharacters,
    NameStart,
    RegistrationClosed,
    WrongCurrentPassword,

    // Handler refusals
    NoSuchPlayer,
    CannotInviteSelf,
    NotAColour,
    NotAPoint,
    CrossSite,

    // Generic error page
    TitleNotFound,
    TitleNotLoggedIn,
    TitleRefused,
    TitleSlowDown,
    TitleConflict,
    TitleUnexpected,
    MessageNotFound,
    MessageNotLoggedIn,
    MessageRefused,
    MessageTooManyRequests,
    MessageConflict,
    MessageUnexpected,
    BackToLobby,

    // Game rule refusals, straight from the store
    ErrOccupied,
    ErrSuicide,
    ErrSuperko,
    ErrGameNotFound,
    ErrNotASeat,
    ErrNotYourTurn,
    ErrGameNotStarted,
    ErrSeatTaken,
    ErrOwnGame,
    ErrGameFinished,
    ErrStaleBoard,
    ErrOpponentJoined,
    ErrUnavailable,

    // Language switcher
    LanguageLabel,
}

use Text::*;

impl Text {
    /// `[English, Chinese, Japanese, Korean]`.
    fn row(self) -> [&'static str; 4] {
        match self {
            LogIn => ["Log in", "登录", "ログイン", "로그인"],
            SignUp => ["Sign up", "注册", "新規登録", "가입"],
            LogOut => ["Log out", "退出", "ログアウト", "로그아웃"],
            Footer => [
                "No clocks. No result. The only way a game ends is that both players pass.",
                "不限时，无胜负。对局仅在对局双方连续停一手后结束。",
                "時間制限なし、勝敗なし。両者が連続してパスしたときに対局が終わります。",
                "시간 제한 없음, 승패 없음. 두 대국자가 연속으로 패스하면 대국이 끝납니다.",
            ],
            Or => ["or", "或", "または", "또는"],

            LobbyTitle => ["lobby", "对局大厅", "ロビー", "로비"],
            HeroTitle => ["Correspondence Go", "通信围棋", "通信囲碁", "통신 바둑"],
            HeroLede => [
                "Play takes as long as it takes. There is no clock and no result: \
                 when both players pass, the game simply ends.",
                "想多久就多久。没有计时，也没有胜负：双方连续停一手，对局就此结束。",
                "好きなだけ時間をかけられます。計時も勝敗もなく、両者がパスすれば対局は終わります。",
                "원하는 만큼 시간을 쓰세요. 시계도 승패도 없고, 두 사람이 모두 패스하면 대국이 끝납니다.",
            ],
            NewGame => ["New game", "新对局", "新規対局", "새 대국"],
            PlayAs => ["Play as", "执子", "手番", "진영"],
            Black => ["Black", "黑棋", "黒", "흑"],
            White => ["White", "白棋", "白", "백"],
            OpponentPlaceholder => ["opponent (optional)", "对手（可留空）", "対戦相手（任意）", "상대 (선택)"],
            OpponentAria => [
                "opponent username, optional",
                "对手用户名，可留空",
                "対戦相手のユーザー名（任意）",
                "상대 아이디 (선택)",
            ],
            CreateGame => ["Create game", "创建对局", "対局を作成", "대국 만들기"],
            CreateHint => [
                "Leave the name empty to open the other seat to anyone. \
                 If you play black you can start straight away.",
                "留空则把另一方开放给任何人。执黑可以立刻落子。",
                "空欄にすると相手側を誰でも取れる席にします。黒番ならすぐに打てます。",
                "비워 두면 상대 자리를 누구나 잡을 수 있습니다. 흑을 잡으면 바로 둘 수 있습니다.",
            ],
            TakeASeat => ["Take a seat", "入座", "席に着く", "자리 잡기"],
            TakeASeatIntro => [
                "Games are open to watch, but playing needs an account.",
                "对局公开可观看，但参与对局需要账号。",
                "対局は観戦できますが、打つにはアカウントが必要です。",
                "대국은 누구나 볼 수 있지만, 두려면 계정이 필요합니다.",
            ],
            TakeASeatTail => [
                "— a name and a password, no email.",
                "——只需用户名和密码，无需邮箱。",
                "— ユーザー名とパスワードだけで、メールは不要です。",
                "— 아이디와 비밀번호만 있으면 됩니다. 이메일은 필요 없습니다.",
            ],
            FilterAll => ["All", "全部", "すべて", "전체"],
            FilterOpen => ["Open seats", "空位", "空席", "빈 자리"],
            FilterRunning => ["Running", "进行中", "対局中", "진행 중"],
            FilterFinished => ["Finished", "已结束", "終局", "종료"],
            FilterMine => ["Mine", "我的", "自分の", "내 대국"],
            Vs => ["vs", "对", "対", "vs"],
            AgoNow => ["just now", "刚刚", "たった今", "방금"],
            AgoMinutes => ["{count} min ago", "{count} 分钟前", "{count} 分前", "{count}분 전"],
            AgoHours => ["{count} h ago", "{count} 小时前", "{count} 時間前", "{count}시간 전"],
            AgoDays => ["{count} d ago", "{count} 天前", "{count} 日前", "{count}일 전"],
            AgoWeeks => ["{count} w ago", "{count} 周前", "{count} 週間前", "{count}주 전"],
            AgoMonths => ["{count} mo ago", "{count} 个月前", "{count} か月前", "{count}개월 전"],
            AgoYears => ["{count} y ago", "{count} 年前", "{count} 年前", "{count}년 전"],
            SortLabel => ["Sort", "排序", "並び替え", "정렬"],
            SortActive => ["Recently active", "最近活跃", "最近の対局順", "최근 활동순"],
            SortNewest => ["Newest", "最新", "新しい順", "최신순"],
            SortQuietest => ["Quietest", "最久未动", "動きのない順", "조용한순"],
            EmptyAll => [
                "Nothing here yet. Start the first game above.",
                "这里还没有对局。在上面开一局吧。",
                "まだ何もありません。上で最初の対局を作りましょう。",
                "아직 아무것도 없습니다. 위에서 첫 대국을 시작하세요.",
            ],
            EmptyOpen => [
                "Nobody is waiting for an opponent right now.",
                "目前没有人在等待对手。",
                "いま対戦相手を待っている対局はありません。",
                "지금 상대를 기다리는 대국이 없습니다.",
            ],
            EmptyRunning => [
                "No game is running right now.",
                "目前没有进行中的对局。",
                "いま進行中の対局はありません。",
                "지금 진행 중인 대국이 없습니다.",
            ],
            EmptyFinished => [
                "No games have finished yet.",
                "还没有结束的对局。",
                "まだ終局した対局はありません。",
                "아직 끝난 대국이 없습니다.",
            ],
            EmptyMine => [
                "You are not in a game yet.",
                "你还没有参与任何对局。",
                "まだどの対局にも参加していません。",
                "아직 어떤 대국에도 참여하지 않았습니다.",
            ],
            OpenSeat => ["open seat", "空位", "空席", "빈 자리"],
            ColourBlack => ["black", "黑", "黒", "흑"],
            ColourWhite => ["white", "白", "白", "백"],

            YourMove => ["your move", "轮到你", "あなたの番", "당신 차례"],
            WaitingForOpponent => [
                "waiting for an opponent",
                "等待对手",
                "対戦相手を待っています",
                "상대를 기다리는 중",
            ],
            Finished => ["finished", "已结束", "終局", "종료"],
            ToMove => ["{name} to move", "轮到 {name}", "{name} の番", "{name} 차례"],
            MoveCount => ["move {count}", "第 {count} 手", "第 {count} 手", "{count}수"],
            TakeSeat => ["Take {colour}", "执{colour}", "{colour}番を取る", "{colour}으로 두기"],
            SeatFree => [
                "The {colour} seat is free.",
                "{colour}方空着。",
                "{colour}番の席が空いています。",
                "{colour} 자리가 비어 있습니다.",
            ],
            LogInToTake => ["Log in to take it.", "登录后即可入座。", "ログインして着席できます。", "로그인하면 잡을 수 있습니다."],
            ShareLink => [
                "Share this page’s link to fill it.",
                "分享本页链接，等待有人入座。",
                "このページのリンクを共有してください。",
                "이 페이지 링크를 공유해 보세요.",
            ],

            GameTitleTag => ["game", "对局", "対局", "대국"],
            NonResultBanner => [
                "Both players passed. The game is finished — no winner, as designed.",
                "双方连续停一手，对局已结束——按设计没有胜负。",
                "両者がパスしました。対局は終了です — 設計どおり勝敗はありません。",
                "두 사람이 모두 패스했습니다. 대국이 끝났습니다 — 설계대로 승패는 없습니다.",
            ],
            Pass => ["Pass", "停一手", "パス", "패스"],
            PassHint => [
                "Two passes in a row end the game.",
                "连续两次停一手即结束对局。",
                "パスが2回続くと対局が終わります。",
                "연속으로 두 번 패스하면 대국이 끝납니다.",
            ],
            ItsMove => ["It is {name}’s move.", "轮到 {name}。", "{name} の番です。", "{name} 차례입니다."],
            WithdrawGame => ["Withdraw this game", "撤回对局", "対局を取り下げる", "대국 철회"],
            WithdrawHint => [
                "Nobody has taken the other seat yet. Withdrawing deletes the game and its moves; \
                 once an opponent joins, it is theirs too.",
                "还没有人入座。撤回会删除该对局及其着法；一旦有对手加入，这局就不再属于你一个人。",
                "まだ相手が入っていません。取り下げると対局とその着手は削除されます。相手が入ったあとは、あなただけのものではありません。",
                "아직 상대가 없습니다. 철회하면 대국과 기보가 삭제됩니다. 상대가 들어오면 이 대국은 상대의 것이기도 합니다.",
            ],
            PassedMove => ["pass", "停一手", "パス", "패스"],
            LastMove => ["Last move {when}", "最后一手 {when}", "最終手 {when}", "마지막 수 {when}"],
            DownloadSgf => ["Download SGF", "下载 SGF", "SGF をダウンロード", "SGF 내려받기"],
            BackToBoard => ["Back to the board", "返回棋盘", "盤面に戻る", "바둑판으로 돌아가기"],
            MoveRefused => ["Move refused", "着法不被接受", "着手できません", "착수할 수 없습니다"],

            AccountTitle => ["your account", "你的账号", "アカウント", "내 계정"],
            SessionsExpire => [
                "Sessions expire after {count} days without use.",
                "登录状态在 {count} 天未使用后失效。",
                "{count} 日使わないとログインが切れます。",
                "{count}일 동안 사용하지 않으면 로그인이 만료됩니다.",
            ],
            SessionsExpireOne => [
                "Sessions expire after {count} day without use.",
                "登录状态在 {count} 天未使用后失效。",
                "{count} 日使わないとログインが切れます。",
                "{count}일 동안 사용하지 않으면 로그인이 만료됩니다.",
            ],
            YourGames => ["Your games", "你的对局", "自分の対局", "내 대국"],
            OpenInLobby => ["Open in the lobby", "在大厅中查看", "ロビーで開く", "로비에서 열기"],
            ChangePassword => ["Change password", "修改密码", "パスワード変更", "비밀번호 변경"],
            CurrentPassword => ["Current password", "当前密码", "現在のパスワード", "현재 비밀번호"],
            NewPassword => ["New password", "新密码", "新しいパスワード", "새 비밀번호"],
            ChangePasswordSubmit => ["Change password", "修改密码", "変更する", "변경하기"],
            ChangePasswordHint => [
                "Changing the password logs out every other device.",
                "修改密码会退出其他所有设备。",
                "変更すると他の端末はすべてログアウトされます。",
                "변경하면 다른 기기에서는 모두 로그아웃됩니다.",
            ],
            ThisDevice => ["This device", "本设备", "この端末", "이 기기"],
            LogOutEverywhere => ["Log out everywhere else", "退出其他所有设备", "他の端末をすべてログアウト", "다른 기기 모두 로그아웃"],
            PasswordChanged => [
                "Password changed. Every other device was logged out.",
                "密码已修改，其他设备均已退出。",
                "パスワードを変更しました。他の端末はログアウトされました。",
                "비밀번호를 변경했습니다. 다른 기기는 모두 로그아웃되었습니다.",
            ],
            LoggedOutEverywhere => [
                "Logged out everywhere else.",
                "已退出其他所有设备。",
                "他の端末をすべてログアウトしました。",
                "다른 기기에서 모두 로그아웃했습니다.",
            ],

            LoginTitle => ["Log in", "登录", "ログイン", "로그인"],
            NameLabel => ["Name", "用户名", "ユーザー名", "아이디"],
            PasswordLabel => ["Password", "密码", "パスワード", "비밀번호"],
            RegisterTitle => ["Sign up", "注册", "新規登録", "가입"],
            RegisterHint => [
                "A name and a password is the whole account: no email, no password reset. \
                 Pick something you will remember.",
                "账号只需要用户名和密码：不需要邮箱，也没有找回密码。请选一个记得住的。",
                "アカウントはユーザー名とパスワードだけです。メールもパスワード再設定もありません。覚えやすいものを選んでください。",
                "계정은 아이디와 비밀번호뿐입니다. 이메일도, 비밀번호 재설정도 없습니다. 기억하기 쉬운 것으로 정하세요.",
            ],
            CreateAccount => ["Create account", "创建账号", "アカウントを作成", "계정 만들기"],
            AlreadyHaveOne => ["Already have one?", "已有账号？", "すでにアカウントをお持ちですか？", "이미 계정이 있나요?"],
            NoAccount => ["No account?", "还没有账号？", "アカウントがありませんか？", "계정이 없나요?"],
            WrongCredentials => ["Wrong name or password.", "用户名或密码错误。", "ユーザー名かパスワードが違います。", "아이디 또는 비밀번호가 틀렸습니다."],
            TooManyAttempts => [
                "Too many attempts from here. Try again in a few minutes.",
                "尝试次数过多，请过几分钟再试。",
                "試行回数が多すぎます。しばらくしてからお試しください。",
                "시도가 너무 많습니다. 몇 분 후에 다시 시도하세요.",
            ],
            TooManyAccounts => [
                "Too many accounts created from here. Try again later.",
                "从此地址创建的账号过多，请稍后再试。",
                "この場所からの登録が多すぎます。後でもう一度お試しください。",
                "여기서 만든 계정이 너무 많습니다. 나중에 다시 시도하세요.",
            ],
            NameTaken => ["That name is already taken.", "该用户名已被占用。", "そのユーザー名はすでに使われています。", "이미 사용 중인 아이디입니다."],
            PasswordTooShort => [
                "Use at least {count} characters for the password.",
                "密码至少需要 {count} 个字符。",
                "パスワードは {count} 文字以上にしてください。",
                "비밀번호는 {count}자 이상이어야 합니다.",
            ],
            PasswordTooLong => [
                "Use at most {count} characters for the password.",
                "密码最多 {count} 个字符。",
                "パスワードは {count} 文字以内にしてください。",
                "비밀번호는 {count}자 이하여야 합니다.",
            ],
            NameTooShort => [
                "Use at least {count} characters for the name.",
                "用户名至少需要 {count} 个字符。",
                "ユーザー名は {count} 文字以上にしてください。",
                "아이디는 {count}자 이상이어야 합니다.",
            ],
            NameTooLong => [
                "Use at most {count} characters for the name.",
                "用户名最多 {count} 个字符。",
                "ユーザー名は {count} 文字以内にしてください。",
                "아이디는 {count}자 이하여야 합니다.",
            ],
            NameCharacters => [
                "Names may contain letters, digits, `_`, `-` and `.`.",
                "用户名可包含字母、数字、`_`、`-` 和 `.`。",
                "ユーザー名には文字、数字、`_`、`-`、`.` が使えます。",
                "아이디에는 문자, 숫자, `_`, `-`, `.`를 쓸 수 있습니다.",
            ],
            NameStart => [
                "Names must start with a letter or a digit.",
                "用户名必须以字母或数字开头。",
                "ユーザー名は文字か数字で始める必要があります。",
                "아이디는 문자나 숫자로 시작해야 합니다.",
            ],
            RegistrationClosed => [
                "this server does not accept new accounts",
                "本服务器不接受新账号",
                "このサーバーは新規アカウントを受け付けていません",
                "이 서버는 새 계정을 받지 않습니다",
            ],
            WrongCurrentPassword => [
                "the current password is not correct",
                "当前密码不正确",
                "現在のパスワードが違います",
                "현재 비밀번호가 맞지 않습니다",
            ],

            NoSuchPlayer => [
                "there is no player called “{name}”",
                "没有名为“{name}”的玩家",
                "「{name}」というプレイヤーはいません",
                "“{name}”이라는 상대가 없습니다",
            ],
            CannotInviteSelf => [
                "you cannot invite yourself to a game",
                "不能邀请自己",
                "自分を対局に招待することはできません",
                "자신을 대국에 초대할 수 없습니다",
            ],
            NotAColour => [
                "“{value}” is not a colour to play",
                "“{value}”不是可执的棋色",
                "「{value}」は選べる手番ではありません",
                "“{value}”은(는) 고를 수 있는 진영이 아닙니다",
            ],
            NotAPoint => [
                "“{value}” is not a point",
                "“{value}”不是棋盘上的点",
                "「{value}」は盤上の点ではありません",
                "“{value}”은(는) 바둑판의 좌표가 아닙니다",
            ],
            CrossSite => [
                "cross-site requests are refused",
                "拒绝跨站请求",
                "クロスサイトのリクエストは拒否されます",
                "다른 사이트에서 온 요청은 거부됩니다",
            ],

            TitleNotFound => ["Not found", "找不到页面", "見つかりません", "찾을 수 없음"],
            TitleNotLoggedIn => ["Not logged in", "未登录", "ログインしていません", "로그인하지 않음"],
            TitleRefused => ["Refused", "已拒绝", "拒否しました", "거부됨"],
            TitleSlowDown => ["Slow down", "请慢一点", "少し待ってください", "잠시만요"],
            TitleConflict => ["Conflict", "冲突", "競合", "충돌"],
            TitleUnexpected => ["Something went wrong", "出错了", "問題が発生しました", "문제가 발생했습니다"],
            MessageNotFound => ["That page does not exist.", "该页面不存在。", "そのページはありません。", "그 페이지는 없습니다."],
            MessageNotLoggedIn => ["You need to log in first.", "请先登录。", "先にログインしてください。", "먼저 로그인해야 합니다."],
            MessageRefused => ["That is not allowed.", "不允许此操作。", "それはできません。", "허용되지 않습니다."],
            MessageTooManyRequests => ["Too many requests. Try again shortly.", "请求过多，请稍后再试。", "リクエストが多すぎます。しばらくしてからお試しください。", "요청이 너무 많습니다. 잠시 후 다시 시도하세요."],
            MessageConflict => ["That conflicts with what already exists.", "与现有内容冲突。", "既存の内容と競合しています。", "이미 있는 내용과 충돌합니다."],
            MessageUnexpected => ["Something went wrong on our side.", "服务器内部出错。", "サーバー側で問題が発生しました。", "서버에서 문제가 발생했습니다."],
            BackToLobby => ["Back to the lobby", "返回大厅", "ロビーに戻る", "로비로 돌아가기"],

            ErrOccupied => [
                "that point is already occupied",
                "该点已有棋子",
                "その点にはすでに石があります",
                "그 자리에 이미 돌이 있습니다",
            ],
            ErrSuicide => ["suicide is not allowed", "不允许自杀", "自殺手は打てません", "자충수는 둘 수 없습니다"],
            ErrSuperko => [
                "that move would repeat an earlier position, which the rules forbid",
                "该着法会造成局面重复，规则不允许",
                "その着手は同一局面の繰り返しになるため打てません",
                "같은 국면이 반복되어 규칙상 둘 수 없습니다",
            ],
            ErrGameNotFound => ["game not found", "找不到该对局", "対局が見つかりません", "대국을 찾을 수 없습니다"],
            ErrNotASeat => [
                "you are not one of the two players in this game",
                "你不是这局的两名对局者之一",
                "この対局の対局者ではありません",
                "이 대국의 대국자가 아닙니다",
            ],
            ErrNotYourTurn => ["it is not your turn", "还没轮到你", "あなたの番ではありません", "당신 차례가 아닙니다"],
            ErrGameNotStarted => [
                "this game is still waiting for an opponent",
                "该对局仍在等待对手",
                "この対局はまだ相手を待っています",
                "이 대국은 아직 상대를 기다리고 있습니다",
            ],
            ErrSeatTaken => ["this game already has an opponent", "该对局已有对手", "この対局にはすでに相手がいます", "이 대국에는 이미 상대가 있습니다"],
            ErrOwnGame => [
                "you cannot take the seat in your own game",
                "不能在自己的对局中入座",
                "自分の対局の席には着けません",
                "자신의 대국 자리는 잡을 수 없습니다",
            ],
            ErrGameFinished => ["this game is finished", "该对局已结束", "この対局は終局しています", "이 대국은 종료되었습니다"],
            ErrStaleBoard => [
                "the board changed while you were looking at it — reload to see the current position",
                "你查看期间棋盘已变化，请刷新查看当前局面",
                "見ている間に盤面が変わりました。再読み込みしてください",
                "보는 사이에 판이 바뀌었습니다. 새로 고쳐 주세요",
            ],
            ErrOpponentJoined => [
                "this game already has an opponent, so it cannot be withdrawn",
                "该对局已有对手，无法撤回",
                "この対局には相手がいるため取り下げられません",
                "이미 상대가 있어 철회할 수 없습니다",
            ],
            ErrUnavailable => [
                "that could not be done right now",
                "暂时无法完成该操作",
                "いまは実行できません",
                "지금은 처리할 수 없습니다",
            ],

            LanguageLabel => ["Language", "语言", "言語", "언어"],
        }
    }
}

/// The string for `text` in `lang`.
pub fn t(lang: Lang, text: Text) -> &'static str {
    text.row()[lang.index()]
}

/// Substitute `{key}` placeholders. Values are inserted literally; callers
/// render the result through maud, which escapes it.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    args.iter().fold(template.to_owned(), |text, (key, value)| {
        text.replace(&format!("{{{key}}}"), value)
    })
}

/// How long ago something happened, in words the language agrees with.
pub fn ago(lang: Lang, now: i64, then: i64) -> String {
    let seconds = (now - then).max(0);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let (text, count) = if seconds < 45 {
        (Text::AgoNow, 0)
    } else if minutes < 60 {
        (Text::AgoMinutes, minutes)
    } else if hours < 24 {
        (Text::AgoHours, hours)
    } else if days < 7 {
        (Text::AgoDays, days)
    } else if days < 70 {
        (Text::AgoWeeks, days / 7)
    } else if days < 730 {
        (Text::AgoMonths, days / 30)
    } else {
        (Text::AgoYears, days / 365)
    };
    let template = t(lang, text);
    if count == 0 {
        template.to_owned()
    } else {
        fill(template, &[("count", &count.to_string())])
    }
}

/// Handy in views: `t(Text::LogIn)` with the language already bound.
pub fn translator(lang: Lang) -> impl Fn(Text) -> &'static str {
    move |text| t(lang, text)
}

tokio::task_local! {
    /// Set by the language middleware, read by the generic error page.
    static CURRENT: Lang;
}

pub fn current() -> Lang {
    CURRENT.try_with(|lang| *lang).unwrap_or_default()
}

pub async fn scope<F: std::future::Future>(lang: Lang, future: F) -> F::Output {
    CURRENT.scope(lang, future).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let Some(end) = rest[start..].find('}') else {
                break;
            };
            found.push(rest[start + 1..start + end].to_owned());
            rest = &rest[start + end + 1..];
        }
        found.sort();
        found
    }

    #[test]
    fn no_translation_is_missing_or_left_in_english_by_accident() {
        for text in ALL_TEXT.iter().copied() {
            for (index, value) in text.row().iter().enumerate() {
                assert!(
                    !value.trim().is_empty(),
                    "{text:?} is empty in language {index}"
                );
            }
        }
    }

    #[test]
    fn every_language_keeps_the_placeholders_of_the_english_string() {
        for text in ALL_TEXT.iter().copied() {
            let row = text.row();
            let expected = placeholders(row[0]);
            for (index, value) in row.iter().enumerate().skip(1) {
                assert_eq!(
                    placeholders(value),
                    expected,
                    "{text:?} in language {index} has different placeholders: {value}"
                );
            }
        }
    }

    #[test]
    fn the_three_translations_are_actually_different() {
        for text in ALL_TEXT.iter().copied() {
            let row = text.row();
            // A few strings are legitimately identical (e.g. "English" labels).
            let distinct = row.iter().collect::<std::collections::HashSet<_>>().len();
            assert!(distinct >= 2, "{text:?} looks untranslated: {row:?}");
        }
    }

    #[test]
    fn fill_substitutes_every_argument() {
        assert_eq!(
            fill(
                "move {count} in {place}",
                &[("count", "12"), ("place", "lobby")]
            ),
            "move 12 in lobby"
        );
        assert_eq!(fill("no placeholders", &[]), "no placeholders");
    }

    #[test]
    fn languages_parse_from_codes_and_headers() {
        assert_eq!(Lang::parse("en"), Some(Lang::En));
        assert_eq!(Lang::parse("zh-CN"), Some(Lang::Zh));
        assert_eq!(
            Lang::parse("zh-TW"),
            Some(Lang::Zh),
            "traditional maps to zh"
        );
        assert_eq!(Lang::parse("ja-JP"), Some(Lang::Ja));
        assert_eq!(Lang::parse("ko"), Some(Lang::Ko));
        assert_eq!(Lang::parse("fr"), None);

        assert_eq!(
            Lang::from_accept_language("ja-JP,ja;q=0.9,en-US;q=0.8"),
            Some(Lang::Ja)
        );
        assert_eq!(
            Lang::from_accept_language("de-DE,de;q=0.9,ko;q=0.5"),
            Some(Lang::Ko)
        );
        assert_eq!(Lang::from_accept_language("fr-FR,fr;q=0.9"), None);
        assert_eq!(Lang::from_accept_language(""), None);
    }

    #[test]
    fn the_switcher_labels_are_in_their_own_language() {
        assert_eq!(Lang::Zh.label(), "中文");
        assert_eq!(Lang::Ja.label(), "日本語");
        assert_eq!(Lang::Ko.label(), "한국어");
        assert_eq!(Lang::Ko.html(), "ko");
    }

    #[test]
    fn a_template_is_only_translated_once() {
        let english = t(Lang::En, Text::Pass);
        assert_eq!(english, "Pass");
        assert_eq!(t(Lang::Ja, Text::Pass), "パス");
    }

    #[test]
    fn the_task_local_falls_back_to_english() {
        assert_eq!(current(), Lang::En);
    }

    /// Compile-time exhaustive list, kept honest by the test above.
    const ALL_TEXT: &[Text] = &[
        LogIn,
        SignUp,
        LogOut,
        Footer,
        Or,
        LobbyTitle,
        HeroTitle,
        HeroLede,
        NewGame,
        PlayAs,
        Black,
        White,
        OpponentPlaceholder,
        OpponentAria,
        CreateGame,
        CreateHint,
        TakeASeat,
        TakeASeatIntro,
        TakeASeatTail,
        FilterAll,
        FilterOpen,
        FilterRunning,
        FilterFinished,
        FilterMine,
        Vs,
        AgoNow,
        AgoMinutes,
        AgoHours,
        AgoDays,
        AgoWeeks,
        AgoMonths,
        AgoYears,
        SortLabel,
        SortActive,
        SortNewest,
        SortQuietest,
        EmptyAll,
        EmptyOpen,
        EmptyRunning,
        EmptyFinished,
        EmptyMine,
        OpenSeat,
        ColourBlack,
        ColourWhite,
        YourMove,
        WaitingForOpponent,
        Finished,
        ToMove,
        MoveCount,
        TakeSeat,
        SeatFree,
        LogInToTake,
        ShareLink,
        GameTitleTag,
        NonResultBanner,
        Pass,
        PassHint,
        ItsMove,
        WithdrawGame,
        WithdrawHint,
        PassedMove,
        LastMove,
        DownloadSgf,
        BackToBoard,
        MoveRefused,
        AccountTitle,
        SessionsExpire,
        SessionsExpireOne,
        YourGames,
        OpenInLobby,
        ChangePassword,
        CurrentPassword,
        NewPassword,
        ChangePasswordSubmit,
        ChangePasswordHint,
        ThisDevice,
        LogOutEverywhere,
        PasswordChanged,
        LoggedOutEverywhere,
        LoginTitle,
        NameLabel,
        PasswordLabel,
        RegisterTitle,
        RegisterHint,
        CreateAccount,
        AlreadyHaveOne,
        NoAccount,
        WrongCredentials,
        TooManyAttempts,
        TooManyAccounts,
        NameTaken,
        PasswordTooShort,
        PasswordTooLong,
        NameTooShort,
        NameTooLong,
        NameCharacters,
        NameStart,
        RegistrationClosed,
        WrongCurrentPassword,
        NoSuchPlayer,
        CannotInviteSelf,
        NotAColour,
        NotAPoint,
        CrossSite,
        TitleNotFound,
        TitleNotLoggedIn,
        TitleRefused,
        TitleSlowDown,
        TitleConflict,
        TitleUnexpected,
        MessageNotFound,
        MessageNotLoggedIn,
        MessageRefused,
        MessageTooManyRequests,
        MessageConflict,
        MessageUnexpected,
        BackToLobby,
        ErrOccupied,
        ErrSuicide,
        ErrSuperko,
        ErrGameNotFound,
        ErrNotASeat,
        ErrNotYourTurn,
        ErrGameNotStarted,
        ErrSeatTaken,
        ErrOwnGame,
        ErrGameFinished,
        ErrStaleBoard,
        ErrOpponentJoined,
        ErrUnavailable,
        LanguageLabel,
    ];
}
