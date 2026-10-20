//! The app's language: English and Ukrainian, chosen in the settings
//! (`language` in `config.toml`, "auto" follows the system). Translations
//! sit next to the code — a table for the window's labels and the [`tr!`]
//! macro for everything the player says while it works — so there are no
//! catalogue files to keep in step and nothing to load at runtime.

/// Which language the UI speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    Uk,
}

impl Lang {
    /// `"uk"` / `"en"`; anything else (including "auto") follows the system.
    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "uk" | "ua" | "ukrainian" => Self::Uk,
            "en" | "english" => Self::En,
            _ => Self::system(),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Uk => "uk",
        }
    }

    /// What the system is set to (`LANG`, `LC_ALL`, or macOS's own list).
    pub fn system() -> Self {
        #[cfg(target_os = "macos")]
        if let Some(lang) = macos_language() {
            return lang;
        }
        for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(value) = std::env::var(key)
                && !value.is_empty()
            {
                return if value.to_ascii_lowercase().starts_with("uk") {
                    Self::Uk
                } else {
                    Self::En
                };
            }
        }
        Self::En
    }

    /// The window's labels in this language.
    pub fn text(self) -> &'static Text {
        match self {
            Self::En => &EN,
            Self::Uk => &UK,
        }
    }
}

/// The user's preferred languages, as macOS keeps them (the environment
/// says nothing about them in an app bundle).
#[cfg(target_os = "macos")]
fn macos_language() -> Option<Lang> {
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()?;
    let list = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
    let first = list
        .split(['"', '\n', ',', '(', ')'])
        .map(str::trim)
        .find(|s| !s.is_empty())?
        .to_owned();
    Some(if first.starts_with("uk") {
        Lang::Uk
    } else {
        Lang::En
    })
}

/// One of the two strings, by language; extra arguments are formatted into
/// it (`tr!(lang, "{} songs", "{} пісень", n)`).
#[macro_export]
macro_rules! tr {
    ($lang:expr, $en:expr, $uk:expr) => {
        match $lang {
            $crate::i18n::Lang::Uk => $uk,
            $crate::i18n::Lang::En => $en,
        }
    };
    ($lang:expr, $en:expr, $uk:expr, $($arg:tt)*) => {
        match $lang {
            $crate::i18n::Lang::Uk => format!($uk, $($arg)*),
            $crate::i18n::Lang::En => format!($en, $($arg)*),
        }
    };
}

/// "пісня / пісні / пісень" by the number before it.
pub fn songs_uk(n: usize) -> &'static str {
    let (tens, ones) = (n % 100, n % 10);
    if (11..=14).contains(&tens) {
        "пісень"
    } else if ones == 1 {
        "пісня"
    } else if (2..=4).contains(&ones) {
        "пісні"
    } else {
        "пісень"
    }
}

/// Like [`tr!`], but always `format!`s — for text with values in it, also
/// when they are captured inline (`trf!(lang, "{n} songs", "{n} пісень")`).
#[macro_export]
macro_rules! trf {
    ($lang:expr, $en:expr, $uk:expr $(, $($arg:tt)*)?) => {
        match $lang {
            $crate::i18n::Lang::Uk => format!($uk $(, $($arg)*)?),
            $crate::i18n::Lang::En => format!($en $(, $($arg)*)?),
        }
    };
}

/// Every label the window shows.
pub struct Text {
    pub sort: &'static str,
    pub move_up: &'static str,
    pub move_down: &'static str,
    pub remove_from_history: &'static str,
    pub save_queue: &'static str,
    pub save_queue_title: &'static str,
    pub clear_history: &'static str,
    pub clear_history_q: &'static str,
    pub clear_history_note: &'static str,
    pub home: &'static str,
    pub explore: &'static str,
    pub library: &'static str,
    pub history: &'static str,
    pub downloads: &'static str,
    pub albums: &'static str,
    pub artists: &'static str,
    pub playlists: &'static str,
    pub songs: &'static str,
    pub account: &'static str,
    pub sign_in: &'static str,
    pub sign_out: &'static str,
    pub sign_in_google: &'static str,
    pub signing_in: &'static str,
    pub syncing: &'static str,
    pub searching: &'static str,
    pub search: &'static str,
    pub search_hint: &'static str,
    pub search_enter: &'static str,
    pub search_prefix: &'static str,
    pub made_with_slint: &'static str,
    pub play: &'static str,
    pub shuffle: &'static str,
    pub radio: &'static str,
    pub save: &'static str,
    pub saved: &'static str,
    pub follow: &'static str,
    pub following: &'static str,
    pub stop: &'static str,
    pub stop_open: &'static str,
    pub delete_all: &'static str,
    pub show_all: &'static str,
    pub songs_suffix: &'static str,
    pub nothing_playing: &'static str,
    pub playing_from: &'static str,
    pub autoplay: &'static str,
    pub up_next: &'static str,
    pub lyrics_tab: &'static str,
    pub related_tab: &'static str,
    pub loading: &'static str,
    pub loading_video: &'static str,
    pub looking_for_lyrics: &'static str,
    pub no_lyrics: &'static str,
    pub nothing_found: &'static str,
    pub empty_playlist: &'static str,
    pub empty_history: &'static str,
    pub empty_downloads: &'static str,
    pub offline_banner: &'static str,
    pub offline_empty: &'static str,
    pub retry: &'static str,
    pub empty_albums: &'static str,
    pub empty_artists: &'static str,
    pub no_match_prefix: &'static str,
    pub no_match_suffix: &'static str,
    pub start_radio: &'static str,
    pub play_next: &'static str,
    pub add_to_queue: &'static str,
    pub download: &'static str,
    pub remove_download: &'static str,
    pub dislike: &'static str,
    pub remove_from_playlist: &'static str,
    pub save_to_playlist: &'static str,
    pub new_playlist_menu: &'static str,
    pub new_playlist: &'static str,
    pub rename_playlist: &'static str,
    pub rename: &'static str,
    pub create: &'static str,
    pub delete: &'static str,
    pub cancel: &'static str,
    pub name: &'static str,
    pub delete_playlist_q_prefix: &'static str,
    pub delete_playlist_q_suffix: &'static str,
    pub delete_playlist_note: &'static str,
    pub delete_downloads_q: &'static str,
    pub delete_downloads_note: &'static str,
    pub sleep_timer: &'static str,
    pub minutes_suffix: &'static str,
    pub end_of_song: &'static str,
    pub turn_off: &'static str,
    pub play_on: &'static str,
    pub this_computer: &'static str,
    pub looking_for_cast: &'static str,
    pub no_cast: &'static str,
    pub settings: &'static str,
    pub language: &'static str,
    pub version_prefix: &'static str,
    pub is_available_suffix: &'static str,
    pub up_to_date: &'static str,
    pub update: &'static str,
    pub updating: &'static str,
    pub check_now: &'static str,
    pub update_to_prefix: &'static str,
    pub autoplay_setting: &'static str,
    pub normalize_setting: &'static str,
    pub crossfade_off: &'static str,
    pub crossfade_prefix: &'static str,
    pub seconds_suffix: &'static str,
    pub notifications_setting: &'static str,
    pub tray_setting: &'static str,
    pub youtube_account: &'static str,
    pub yt_signed_in: &'static str,
    pub yt_sign_in_window: &'static str,
    pub yt_sign_in_import: &'static str,
    pub import: &'static str,
    pub pick_browser: &'static str,
    pub browser: &'static str,
    pub or_browser_sign_in: &'static str,
    pub browser_note: &'static str,
    pub off: &'static str,
    pub integrations_note: &'static str,
    pub google_account: &'static str,
    pub yt_signed_in_long: &'static str,
    pub yt_sign_in_long_window: &'static str,
    pub yt_sign_in_long_import: &'static str,
    pub data_api_title: &'static str,
    pub data_api_connected: &'static str,
    pub data_api_not_needed: &'static str,
    pub data_api_optional: &'static str,
    pub set_up: &'static str,
    pub hide: &'static str,
    pub oauth_client_prefix: &'static str,
    pub oauth_step1: &'static str,
    pub oauth_step2: &'static str,
    pub oauth_how: &'static str,
    pub open_google_cloud: &'static str,
    pub import_downloaded_json: &'static str,
    pub or_paste: &'static str,
    pub client_id_hint: &'static str,
    pub client_secret_hint: &'static str,
    pub save_client: &'static str,
    pub client_set: &'static str,
    pub open_browser_again: &'static str,
    pub waiting_browser: &'static str,
    pub unverified_note: &'static str,
}

pub static EN: Text = Text {
    sort: "Sort",
    move_up: "Move up",
    move_down: "Move down",
    remove_from_history: "Remove from history",
    save_queue: "Save as playlist",
    save_queue_title: "Save the queue as a playlist",
    clear_history: "Clear history",
    clear_history_q: "Clear the history?",
    clear_history_note: "Play counts are forgotten too.",
    home: "Home",
    explore: "Explore",
    library: "Library",
    history: "History",
    downloads: "Downloads",
    albums: "Albums",
    artists: "Artists",
    playlists: "Playlists",
    songs: "Songs",
    account: "Account",
    sign_in: "Sign in",
    sign_out: "Sign out",
    sign_in_google: "Sign in with Google",
    signing_in: "Signing in…",
    syncing: "Syncing…",
    searching: "Searching…",
    search: "Search",
    search_hint: "Search YouTube Music, or type to filter",
    search_enter: "Enter ↵ search",
    search_prefix: "Search: ",
    made_with_slint: "Made with Slint",
    play: "Play",
    shuffle: "Shuffle",
    radio: "Radio",
    save: "Save",
    saved: "Saved",
    follow: "Follow",
    following: "Following",
    stop: "Stop",
    stop_open: "Stop (",
    delete_all: "Delete all",
    show_all: "Show all ",
    songs_suffix: " songs",
    nothing_playing: "Nothing playing",
    playing_from: "Playing from",
    autoplay: "Autoplay",
    up_next: "UP NEXT",
    lyrics_tab: "LYRICS",
    related_tab: "RELATED",
    loading: "Loading…",
    loading_video: "Loading the video…",
    looking_for_lyrics: "Looking for lyrics…",
    no_lyrics: "No lyrics for this song",
    nothing_found: "Nothing found",
    empty_playlist: "This playlist is empty",
    empty_history: "Songs you play show up here",
    empty_downloads: "Downloaded songs play offline — download from a song's ⋮ menu",
    offline_banner: "No internet connection · downloaded songs still play",
    offline_empty: "You're offline. This needs the internet — your downloads still play.",
    retry: "Retry",
    empty_albums: "Save albums from their page",
    empty_artists: "Follow artists from their page (sync brings your subscriptions)",
    no_match_prefix: "No songs match \"",
    no_match_suffix: "\" — Enter searches YouTube Music",
    start_radio: "Start radio",
    play_next: "Play next",
    add_to_queue: "Add to queue",
    download: "Download",
    remove_download: "Remove download",
    dislike: "Dislike",
    remove_from_playlist: "Remove from playlist",
    save_to_playlist: "Save to playlist",
    new_playlist_menu: "New playlist…",
    new_playlist: "New playlist",
    rename_playlist: "Rename playlist",
    rename: "Rename",
    create: "Create",
    delete: "Delete",
    cancel: "Cancel",
    name: "Name",
    delete_playlist_q_prefix: "Delete “",
    delete_playlist_q_suffix: "”?",
    delete_playlist_note: "The playlist is deleted on YouTube too.",
    delete_downloads_q: "Delete all downloaded songs?",
    delete_downloads_note: "Their files are removed from this computer; the songs stay in your library and play online.",
    sleep_timer: "Sleep timer",
    minutes_suffix: " minutes",
    end_of_song: "End of song",
    turn_off: "Turn off",
    play_on: "Play on",
    this_computer: "This computer",
    looking_for_cast: "Looking for Chromecasts…",
    no_cast: "No Chromecasts found",
    settings: "Settings",
    language: "Language",
    version_prefix: "Version ",
    is_available_suffix: " is available",
    up_to_date: "Up to date (checked at start)",
    update: "Update",
    updating: "Updating…",
    check_now: "Check now",
    update_to_prefix: "Update to ",
    autoplay_setting: "Autoplay: continue with similar songs",
    normalize_setting: "Normalize volume (like YouTube Music)",
    crossfade_off: "Crossfade: off (gapless)",
    crossfade_prefix: "Crossfade: ",
    seconds_suffix: " s",
    notifications_setting: "Notify when the song changes",
    tray_setting: "Icon in the menu bar / tray (next start)",
    youtube_account: "YouTube account",
    yt_signed_in: "Signed in. Age-restricted songs and your YouTube Music use this account.",
    yt_sign_in_window: "For age-restricted songs and your YouTube Music. Google's own page opens in a window; the password goes only to Google.",
    yt_sign_in_import: "For age-restricted songs and your YouTube Music: sign in at youtube.com in a browser, pick it below, then Import.",
    import: "Import",
    pick_browser: "Pick browser",
    browser: "Browser",
    or_browser_sign_in: "Or use a browser's sign-in",
    browser_note: "Use the YouTube sign-in of this browser, only for such songs. macOS: needs Full Disk Access for Yarek Music (System Settings → Privacy & Security).",
    off: "Off",
    integrations_note: "Last.fm and Discord: see config.toml (README: Integrations).",
    google_account: "Google account",
    yt_signed_in_long: "✓ Signed in to YouTube. Your library, age-restricted songs and your YouTube Music use this account.",
    yt_sign_in_long_window: "Sign in for your library (liked music, playlists), age-restricted songs and your own YouTube Music. Google's own page opens in a window; the password goes only to Google.",
    yt_sign_in_long_import: "Sign in at youtube.com in a browser on this computer, pick that browser and Import its sign-in (your library, age-restricted songs, your YouTube Music).",
    data_api_title: "Library sync (YouTube Data API)",
    data_api_connected: "Connected.",
    data_api_not_needed: "Not needed: your library syncs through the sign-in above.",
    data_api_optional: "Optional: liked music and playlists through your own Google OAuth client. The sign-in above does it without one.",
    set_up: "Set up",
    hide: "Hide",
    oauth_client_prefix: "OAuth client: ",
    oauth_step1: "1. Your Google OAuth client",
    oauth_step2: "2. Sign in",
    oauth_how: "In Google Cloud: enable YouTube Data API v3, add your account as a test user, create a client of type “Desktop app” and download its JSON (README: “Set up Google sign-in”).",
    open_google_cloud: "Open Google Cloud",
    import_downloaded_json: "Import downloaded JSON",
    or_paste: "…or paste it:",
    client_id_hint: "Client ID (…apps.googleusercontent.com)",
    client_secret_hint: "Client secret (GOCSPX-…)",
    save_client: "Save client",
    client_set: "✓ client set",
    open_browser_again: "Open the browser again",
    waiting_browser: "Waiting for your browser…",
    unverified_note: "Google warns the app is unverified: it's your own client, choose Continue.",
};

pub static UK: Text = Text {
    sort: "Сортувати",
    move_up: "Вище",
    move_down: "Нижче",
    remove_from_history: "Прибрати з історії",
    save_queue: "Зберегти як плейлист",
    save_queue_title: "Зберегти чергу як плейлист",
    clear_history: "Очистити історію",
    clear_history_q: "Очистити історію?",
    clear_history_note: "Лічильники прослуховувань теж скинуться.",
    home: "Головна",
    explore: "Огляд",
    library: "Бібліотека",
    history: "Історія",
    downloads: "Завантаження",
    albums: "Альбоми",
    artists: "Виконавці",
    playlists: "Плейлисти",
    songs: "Пісні",
    account: "Акаунт",
    sign_in: "Увійти",
    sign_out: "Вийти",
    sign_in_google: "Увійти через Google",
    signing_in: "Входжу…",
    syncing: "Синхронізую…",
    searching: "Шукаю…",
    search: "Пошук",
    search_hint: "Пошук у YouTube Music або фільтр списку",
    search_enter: "Enter ↵ шукати",
    search_prefix: "Пошук: ",
    made_with_slint: "Зроблено на Slint",
    play: "Слухати",
    shuffle: "Перемішати",
    radio: "Радіо",
    save: "Зберегти",
    saved: "Збережено",
    follow: "Стежити",
    following: "Стежу",
    stop: "Зупинити",
    stop_open: "Зупинити (",
    delete_all: "Видалити все",
    show_all: "Показати всі ",
    songs_suffix: " пісень",
    nothing_playing: "Нічого не грає",
    playing_from: "Грає з",
    autoplay: "Автоплей",
    up_next: "ДАЛІ",
    lyrics_tab: "ТЕКСТ",
    related_tab: "СХОЖЕ",
    loading: "Завантажую…",
    loading_video: "Завантажую відео…",
    looking_for_lyrics: "Шукаю текст…",
    no_lyrics: "Для цієї пісні немає тексту",
    nothing_found: "Нічого не знайдено",
    empty_playlist: "Цей плейлист порожній",
    empty_history: "Тут з'являються пісні, які ти слухав",
    empty_downloads: "Завантажені пісні грають офлайн — завантаж із меню ⋮ пісні",
    offline_banner: "Немає інтернету · завантажені пісні грають",
    offline_empty: "Немає інтернету. Тут потрібен інтернет — завантажені пісні грають і без нього.",
    retry: "Ще раз",
    empty_albums: "Зберігай альбоми з їхньої сторінки",
    empty_artists: "Став стеження за виконавцями на їхній сторінці (синхронізація підтягне підписки)",
    no_match_prefix: "Немає пісень за запитом \"",
    no_match_suffix: "\" — Enter шукає в YouTube Music",
    start_radio: "Увімкнути радіо",
    play_next: "Грати наступною",
    add_to_queue: "У чергу",
    download: "Завантажити",
    remove_download: "Видалити завантаження",
    dislike: "Не подобається",
    remove_from_playlist: "Прибрати з плейлиста",
    save_to_playlist: "Зберегти в плейлист",
    new_playlist_menu: "Новий плейлист…",
    new_playlist: "Новий плейлист",
    rename_playlist: "Перейменувати плейлист",
    rename: "Перейменувати",
    create: "Створити",
    delete: "Видалити",
    cancel: "Скасувати",
    name: "Назва",
    delete_playlist_q_prefix: "Видалити «",
    delete_playlist_q_suffix: "»?",
    delete_playlist_note: "Плейлист видалиться і на YouTube.",
    delete_downloads_q: "Видалити всі завантажені пісні?",
    delete_downloads_note: "Файли зникнуть із цього комп'ютера; пісні лишаться в бібліотеці й гратимуть онлайн.",
    sleep_timer: "Таймер сну",
    minutes_suffix: " хв",
    end_of_song: "До кінця пісні",
    turn_off: "Вимкнути",
    play_on: "Грати на",
    this_computer: "Цей комп'ютер",
    looking_for_cast: "Шукаю Chromecast…",
    no_cast: "Chromecast не знайдено",
    settings: "Налаштування",
    language: "Мова",
    version_prefix: "Версія ",
    is_available_suffix: " доступна",
    up_to_date: "Версія свіжа (перевірено на старті)",
    update: "Оновити",
    updating: "Оновлюю…",
    check_now: "Перевірити",
    update_to_prefix: "Оновити до ",
    autoplay_setting: "Автоплей: продовжувати схожими піснями",
    normalize_setting: "Вирівнювати гучність (як у YouTube Music)",
    crossfade_off: "Перехід: вимкнено (без пауз)",
    crossfade_prefix: "Перехід: ",
    seconds_suffix: " с",
    notifications_setting: "Сповіщати про зміну пісні",
    tray_setting: "Іконка в меню-барі / треї (з наступного запуску)",
    youtube_account: "Акаунт YouTube",
    yt_signed_in: "Вхід виконано. Пісні 18+ і твій YouTube Music ідуть через цей акаунт.",
    yt_sign_in_window: "Для пісень 18+ і твого YouTube Music. Сторінка Google відкриється у вікні; пароль іде лише до Google.",
    yt_sign_in_import: "Для пісень 18+ і твого YouTube Music: увійди на youtube.com у браузері, обери його нижче і натисни «Імпорт».",
    import: "Імпорт",
    pick_browser: "Обрати браузер",
    browser: "Браузер",
    or_browser_sign_in: "Або взяти вхід із браузера",
    browser_note: "Брати вхід у YouTube із цього браузера, лише для таких пісень. macOS: потрібен Full Disk Access для Yarek Music (Системні параметри → Конфіденційність і безпека).",
    off: "Вимк.",
    integrations_note: "Last.fm і Discord: дивись config.toml (README: Integrations).",
    google_account: "Акаунт Google",
    yt_signed_in_long: "✓ Вхід у YouTube виконано. Бібліотека, пісні 18+ і твій YouTube Music ідуть через цей акаунт.",
    yt_sign_in_long_window: "Увійди, щоб отримати свою бібліотеку (вподобані, плейлисти), пісні 18+ і власний YouTube Music. Сторінка Google відкриється у вікні; пароль іде лише до Google.",
    yt_sign_in_long_import: "Увійди на youtube.com у браузері на цьому комп'ютері, обери той браузер і натисни «Імпорт» (бібліотека, пісні 18+, твій YouTube Music).",
    data_api_title: "Синхронізація бібліотеки (YouTube Data API)",
    data_api_connected: "Підключено.",
    data_api_not_needed: "Не потрібно: бібліотека синхронізується через вхід вище.",
    data_api_optional: "Необов'язково: вподобане й плейлисти через власний OAuth-клієнт Google. Вхід вище робить це без нього.",
    set_up: "Налаштувати",
    hide: "Сховати",
    oauth_client_prefix: "OAuth-клієнт: ",
    oauth_step1: "1. Твій OAuth-клієнт Google",
    oauth_step2: "2. Вхід",
    oauth_how: "У Google Cloud: увімкни YouTube Data API v3, додай свій акаунт як тестового користувача, створи клієнт типу «Desktop app» і завантаж його JSON (README: «Set up Google sign-in»).",
    open_google_cloud: "Відкрити Google Cloud",
    import_downloaded_json: "Імпортувати завантажений JSON",
    or_paste: "…або встав вручну:",
    client_id_hint: "Client ID (…apps.googleusercontent.com)",
    client_secret_hint: "Client secret (GOCSPX-…)",
    save_client: "Зберегти клієнт",
    client_set: "✓ клієнт збережено",
    open_browser_again: "Відкрити браузер ще раз",
    waiting_browser: "Чекаю на браузер…",
    unverified_note: "Google попередить, що застосунок неперевірений: це твій власний клієнт, тисни «Продовжити».",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_say_something() {
        assert_eq!(Lang::from_code("uk"), Lang::Uk);
        assert_eq!(Lang::from_code("en"), Lang::En);
        assert_eq!(Lang::Uk.code(), "uk");
        // Nothing left untranslated or empty.
        let (en, uk) = (Lang::En.text(), Lang::Uk.text());
        assert!(!en.home.is_empty() && !uk.home.is_empty());
        assert_ne!(en.settings, uk.settings);
    }

    #[test]
    fn ukrainian_counts_songs_properly() {
        let s = |n| format!("{n} {}", songs_uk(n));
        assert_eq!(s(1), "1 пісня");
        assert_eq!(s(3), "3 пісні");
        assert_eq!(s(5), "5 пісень");
        assert_eq!(s(11), "11 пісень");
        assert_eq!(s(21), "21 пісня");
        assert_eq!(s(102), "102 пісні");
    }

    #[test]
    fn tr_picks_the_language() {
        assert_eq!(tr!(Lang::Uk, "Play", "Слухати"), "Слухати");
        assert_eq!(tr!(Lang::En, "Play", "Слухати"), "Play");
        assert_eq!(tr!(Lang::Uk, "{n} songs", "{n} пісень", n = 3), "3 пісень");
    }
}
