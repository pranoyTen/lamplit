// Lectern, native: the Slint screens over the playback core. Network calls run on worker threads and hand their results back to the UI
// thread with invoke_from_event_loop; everything the UI owns lives in APP on that thread.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod api;
mod book;
mod covers;
mod downloads;
mod eq;
mod http_source;
mod library;
mod media;
mod metered;
mod mini;
mod player;
mod stats;
mod stretch;
mod updater;

use api::{Book, Client, Details};
use book::{Bookmark, Chapter, Sleep};
use covers::Covers;
use downloads::Downloads;
use library::{Filter, Sort};
use player::Engine;
use slint::{Color, ComponentHandle, Image, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use souvlaki::{MediaControlEvent, SeekDirection};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

slint::include_modules!();

struct Playing {
    engine: Engine,
    session: Option<String>, // None when the server can't be reached: the place is then remembered on this computer only
    id: String,
    ended: bool,
    title: String,
    duration: f64,
    listened: f64, // seconds heard since the last report to the server
    last_tick: Instant,
    chapters: Vec<Chapter>,
    bookmarks: Vec<Bookmark>,
    shown_chapter: Option<Option<usize>>, // the chapter the window last showed, so lists are rebuilt only on a change
    sleep: Sleep,
}

struct App {
    ui: slint::Weak<AppWindow>,
    _sink: rodio::MixerDeviceSink,
    out: rodio::Player,
    client: Option<Client>,
    books: Vec<Book>,
    in_progress: Vec<Book>,
    progress: HashMap<String, f64>,
    library: Rc<VecModel<BookCard>>,
    continuing: Rc<VecModel<BookCard>>,
    recent: Rc<VecModel<BookCard>>,
    series: Vec<api::Series>,
    series_model: Rc<VecModel<BookCard>>,
    series_books: Rc<VecModel<BookCard>>,
    finished_model: Rc<VecModel<Recent>>,
    listening: Option<(Instant, api::Listening)>, // the stats, and when they were fetched
    covers: Covers,
    playing: Option<Playing>,
    speed: f64,
    media: Option<souvlaki::MediaControls>,
    shown: (Option<bool>, Instant), // the play state last published to the OS overlay and the tray, and when
    dl: Downloads,
    downloaded: HashSet<String>, // finished downloads, for the Library's Downloaded filter
    prefs: Prefs,
    places: HashMap<String, f64>, // where each book was left, kept here for listening offline
    update: Option<std::sync::Arc<cargo_packager_updater::Update>>, // a newer release found by the last check
    libs: Vec<(String, String)>, // the book libraries: id and name
    gains: Arc<eq::Gains>,
    eq_model: Rc<VecModel<f32>>,
    paused_at: Option<Instant>, // when playback was last paused, for the rewind after a long pause
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Prefs {
    dark: bool,
    speed: f64,
    folder: Option<std::path::PathBuf>, // where new downloads go; None is the app's own data folder
    limit_gb: u64,                      // 0 = no limit
    auto_remove: bool,
    back: u32,
    fwd: u32,
    rewind: bool,
    auto_next: bool,
    eq: [f32; 5],
    gh_token: String, // shortcut: kept in the settings file like the Tauri app kept it; drop it once the repository is public
    speeds: HashMap<String, f64>, // a speed picked on the player for one book wins over the default `speed`
    sleep: i32,                   // the default sleep timer: 0 off, 1-3 for 15, 30 or 45 minutes, 4 end of chapter
    tray: bool,                   // closing the window keeps Lectern in the tray; off, closing quits
    sync: bool,                   // off: no progress reports, and books start where this computer left them
    metered: bool,                // download on metered connections too
    hidden_libs: Vec<String>,
    volume: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            dark: false, speed: 1.0, folder: None, limit_gb: 10, auto_remove: false, back: 15, fwd: 30, rewind: true, auto_next: true,
            eq: [0.0; 5], gh_token: String::new(), speeds: HashMap::new(), sleep: 0, tray: true, sync: true, metered: false,
            hidden_libs: vec![], volume: 1.0,
        }
    }
}

// downloads on a metered connection; read by the download worker
static METERED_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const SLEEPS: [f64; 3] = [15.0, 30.0, 45.0];
const LIMITS_GB: [u64; 5] = [5, 10, 20, 50, 0];
const BACKS: [u32; 3] = [10, 15, 30];
const FWDS: [u32; 3] = [15, 30, 60];

/// The preset these gains match, or 5 (Custom).
fn eq_preset(db: [f32; 5]) -> i32 {
    eq::PRESETS.iter().position(|(_, p)| *p == db).unwrap_or(5) as i32
}

/// Applies new equalizer gains to the playing audio and saves them. `redraw` replaces the sliders' list (a preset was picked);
/// a moved slider only updates its own row, so the drag isn't interrupted.
fn set_eq(db: [f32; 5], redraw: bool) {
    let model = with_app(|a| {
        a.prefs.eq = db;
        a.gains.set(db);
        if redraw {
            a.eq_model = Rc::new(VecModel::from(db.to_vec()));
        } else {
            for (i, g) in db.iter().enumerate() {
                if a.eq_model.row_data(i) != Some(*g) {
                    a.eq_model.set_row_data(i, *g);
                }
            }
        }
        a.eq_model.clone()
    });
    save_prefs();
    if let Some(ui) = window() {
        let st = ui.global::<State>();
        if redraw {
            st.set_eq(model.into());
        }
        st.set_eq_preset(eq_preset(db));
    }
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(a.borrow_mut().as_mut().expect("the app is set up before the window runs")))
}

fn window() -> Option<AppWindow> {
    with_app(|a| a.ui.upgrade())
}

// the sleep timer's clock: seconds since the app started
fn now_s() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

fn clock(t: f64) -> String {
    let t = t.max(0.0) as u64;
    let (h, m, s) = (t / 3600, t % 3600 / 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Runs `work` on a worker thread and `done` with its result back on the UI thread.
fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + Send + 'static) {
    std::thread::spawn(move || {
        let r = work();
        let _ = slint::invoke_from_event_loop(move || done(r));
    });
}

fn say(msg: impl Into<SharedString>) {
    if let Some(ui) = window() {
        ui.global::<State>().set_message(msg.into());
    }
}

// ---- saved between runs ----
// shortcut: the token sits in a plain file in the user's config folder, as it did in the Tauri app's local storage; move it to the OS
// keychain (keyring crate) before release
fn config_dir() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("lectern-native"))
}

fn save_session(c: Option<(&Client, &str)>) {
    let Some(d) = config_dir() else { return };
    let p = d.join("session.json");
    let _ = match c {
        Some((c, user)) => std::fs::create_dir_all(&d)
            .and_then(|_| std::fs::write(&p, serde_json::json!({ "server": c.base, "token": c.token, "username": user }).to_string())),
        None => std::fs::remove_file(&p),
    };
}

/// The saved sign-in: the client and the username.
fn load_session() -> Option<(Client, String)> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config_dir()?.join("session.json")).ok()?).ok()?;
    Some((Client::new(v["server"].as_str()?, v["token"].as_str()?).ok()?, v["username"].as_str().unwrap_or_default().to_string()))
}

fn save_json(name: &str, v: &impl serde::Serialize) {
    if let Some(d) = config_dir() {
        let _ = std::fs::create_dir_all(&d).and_then(|_| std::fs::write(d.join(name), serde_json::to_string(v).unwrap_or_default()));
    }
}

fn load_json<T: serde::de::DeserializeOwned>(name: &str) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(config_dir()?.join(name)).ok()?).ok()
}

fn save_prefs() {
    save_json("prefs.json", &with_app(|a| a.prefs.clone()));
}

/// Where downloads go when no folder has been chosen.
fn default_folder() -> std::path::PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("lectern-native").join("downloads")
}

/// The library as last loaded, so Home, the Library and downloaded books still work without a connection.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Offline {
    books: Vec<Book>,
    in_progress: Vec<Book>,
    progress: HashMap<String, f64>,
    #[serde(default)]
    libs: Vec<(String, String)>, // id and name
}

/// The tray icon as RGBA pixels, decoded from the PNG built into the exe.
fn tray_icon() -> Option<Image> {
    png_image(include_bytes!("../ui/icons/tray.png"))
}

/// The window icon (title bar, taskbar, Alt+Tab), decoded the same way.
fn app_icon() -> Option<Image> {
    png_image(include_bytes!("../icons/128x128.png"))
}

/// A PNG built into the exe as RGBA pixels: images embedded for the software renderer can't become window or tray icons.
fn png_image(bytes: &[u8]) -> Option<Image> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let px = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&buf[..info.buffer_size()], info.width, info.height);
    Some(Image::from_rgba8(px))
}

// ---- books and covers ----
fn tint(title: &str) -> Color {
    let (r, g, b) = library::tint(title);
    Color::from_rgb_u8(r, g, b)
}

fn card(a: &mut App, b: &Book) -> BookCard {
    BookCard {
        id: b.id.clone().into(),
        cover_id: b.id.clone().into(),
        title: b.title.clone().into(),
        author: b.author.clone().into(),
        meta: if b.duration > 0.0 { format!("{} · {}", b.author, library::length(b.duration)) } else { b.author.clone() }.into(),
        progress: a.progress.get(&b.id).copied().unwrap_or(0.0) as f32,
        cover: a.covers.get(&b.id).unwrap_or_default(),
        tint: tint(&b.title),
    }
}

fn find_book(a: &App, id: &str) -> Option<Book> {
    a.books.iter().chain(&a.in_progress).find(|b| b.id == id).cloned()
}

fn want_cover(id: &str) {
    with_app(|a| if let Some(c) = &a.client { a.covers.want(&c.clone(), id) });
}

// a cover arrived: put it into every place that shows that book, and take covers pushed out of the cache off their rows
fn cover_arrived(id: String, px: Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>>) {
    let Some(px) = px else { return with_app(|a| a.covers.failed(&id)) };
    let img = Image::from_rgba8(px);
    let evicted = with_app(|a| a.covers.insert(id.clone(), img.clone()));
    let set = |model: &VecModel<BookCard>, id: &str, img: &Image| {
        for i in 0..model.row_count() {
            if let Some(mut r) = model.row_data(i).filter(|r| r.cover_id == id) {
                r.cover = img.clone();
                model.set_row_data(i, r);
            }
        }
    };
    with_app(|a| {
        for m in [&a.library, &a.continuing, &a.recent, &a.series_model, &a.series_books] {
            set(m, &id, &img);
            for old in &evicted {
                set(m, old, &Image::default());
            }
        }
        for i in 0..a.finished_model.row_count() {
            if let Some(mut r) = a.finished_model.row_data(i).filter(|r| r.id == id) {
                r.cover = img.clone();
                a.finished_model.set_row_data(i, r);
            }
        }
    });
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    let mut hero = st.get_hero();
    if hero.id == id {
        hero.cover = img.clone();
        st.set_hero(hero);
    }
    let mut book = st.get_book();
    if book.id == id {
        book.cover = img.clone();
        st.set_book(book);
    }
    if st.get_now_id() == id {
        st.set_now_cover(img);
    }
}

fn covers_needed(list: &str, first: i32, last: i32) {
    if first < 0 {
        return;
    }
    let ids: Vec<String> = with_app(|a| {
        let model = if list == "series" { &a.series_model } else { &a.library };
        (first..=last).filter_map(|i| model.row_data(i as usize)).map(|r| r.cover_id.to_string()).filter(|id| !id.is_empty()).collect()
    });
    for id in ids.iter().rev() {
        want_cover(id); // newest request is served first, so the top of the screen is asked for last
    }
}

// ---- series ----
fn show_series() {
    if with_app(|a| !a.series.is_empty()) {
        return fill_series();
    }
    let Some(client) = with_app(|a| a.client.clone()) else { return };
    if let Some(ui) = window() {
        ui.global::<State>().set_series_loading(true);
    }
    background(move || client.series(), |r| {
        if let Some(ui) = window() {
            ui.global::<State>().set_series_loading(false);
        }
        match r {
            Ok(s) => {
                with_app(|a| a.series = s);
                fill_series();
            }
            Err(e) => say(e),
        }
    });
}

fn fill_series() {
    with_app(|a| {
        let series: Vec<api::Series> = a.series.iter().filter(|s| s.books.iter().any(|b| shown(a, b))).cloned().collect(); // series of hidden libraries go too
        let rows: Vec<BookCard> = series.iter()
            .map(|s| BookCard {
                id: s.id.clone().into(),
                cover_id: s.books.first().map(|b| b.id.clone()).unwrap_or_default().into(),
                title: s.name.clone().into(),
                author: SharedString::new(),
                meta: format!("{} {}", s.books.len(), if s.books.len() == 1 { "book" } else { "books" }).into(),
                progress: 0.0,
                cover: s.books.first().and_then(|b| a.covers.get(&b.id)).unwrap_or_default(),
                tint: tint(&s.name),
            })
            .collect();
        a.series_model.set_vec(rows);
    });
    covers_needed("series", 0, 23);
}

fn open_series(id: String) {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    let Some(s) = with_app(|a| a.series.iter().find(|s| s.id == id).cloned()) else { return };
    st.set_series_title(s.name.clone().into());
    st.set_series_count(format!("{} {}", s.books.len(), if s.books.len() == 1 { "book" } else { "books" }).into());
    with_app(|a| {
        let rows: Vec<BookCard> = s.books.iter().map(|b| card(a, b)).collect();
        a.series_books.set_vec(rows);
    });
    for b in s.books.iter().rev() {
        want_cover(&b.id);
    }
    st.set_route("series-books".into());
}

// ---- stats ----
fn hours(secs: f64) -> String {
    if secs >= 36000.0 { format!("{}h", (secs / 3600.0).round()) } else { library::length(secs) }
}

/// Fetches the listening stats (at most once a minute) and fills the Stats page for the chosen range.
fn show_stats() {
    let fresh = with_app(|a| a.listening.as_ref().is_some_and(|(t, _)| t.elapsed() < Duration::from_secs(60)));
    if fresh {
        return fill_stats();
    }
    let Some(client) = with_app(|a| a.client.clone()) else { return };
    if let Some(ui) = window() {
        ui.global::<State>().set_stats_loading(true);
    }
    background(move || client.listening(), |r| {
        if let Some(ui) = window() {
            ui.global::<State>().set_stats_loading(false);
        }
        match r {
            Ok(l) => {
                with_app(|a| a.listening = Some((Instant::now(), l)));
                fill_stats();
            }
            Err(e) => say(e),
        }
    });
}

fn fill_stats() {
    use chrono::TimeZone;
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    let Some(l) = with_app(|a| a.listening.as_ref().map(|(_, l)| l.clone())) else { return };
    let range = [stats::Range::Week, stats::Range::Month, stats::Range::Year][st.get_stats_range().clamp(0, 2) as usize];
    let now = chrono::Local::now();
    let local_day = |ms: f64| chrono::Local.timestamp_millis_opt(ms as i64).single().map(|t| t.date_naive());
    let authors: Vec<(String, f64)> = l.items.iter().map(|i| (i.author.clone(), i.secs)).collect();
    let finished: Vec<chrono::NaiveDate> = l.finished.iter().filter_map(|(_, at)| local_day(*at)).collect();
    let s = stats::summarize(&l.days, &authors, &finished, range, now.date_naive());

    st.set_stats_subtitle(match range {
        stats::Range::Week => "Listening over the last 7 days".to_string(),
        stats::Range::Month => "Listening so far this month".to_string(),
        stats::Range::Year => format!("Listening so far in {}", chrono::Datelike::year(&now)),
    }.into());
    let tiles = vec![
        Tile { label: "Time listened".into(), value: hours(s.time).into() },
        Tile { label: "Books finished".into(), value: s.finished.to_string().into() },
        Tile { label: "Daily average".into(), value: library::length(s.avg).into() },
        Tile { label: "Current streak".into(), value: format!("{} {}", s.streak, if s.streak == 1 { "day" } else { "days" }).into() },
    ];
    st.set_tiles(ModelRc::new(VecModel::from(tiles)));
    let year = range == stats::Range::Year;
    st.set_chart_title(if year { "Hours listened per month" } else { "Minutes listened per day" }.into());
    let max = s.bars.iter().map(|b| b.1).fold(1.0, f64::max);
    let n = s.bars.len();
    let bars: Vec<BarItem> = s.bars.iter().enumerate()
        .map(|(i, (label, secs))| BarItem {
            label: label.clone().into(),
            value: (secs / max) as f32,
            text: if year { format!("{}h", (secs / 3600.0).round()) } else { format!("{}m", (secs / 60.0).round()) }.into(),
            current: i + 1 == n,
        })
        .collect();
    st.set_chart(ModelRc::new(VecModel::from(bars)));
    let top = s.authors.first().map_or(1.0, |a| a.1.max(1.0));
    let authors: Vec<BarItem> = s.authors.iter()
        .map(|(name, secs)| BarItem { label: name.clone().into(), value: (secs / top) as f32, text: hours(*secs).into(), current: false })
        .collect();
    st.set_top_authors(ModelRc::new(VecModel::from(authors)));

    // the five most recently finished books, named from the stats or the library
    let mut done = l.finished.clone();
    done.sort_by(|a, b| b.1.total_cmp(&a.1));
    let recent: Vec<Recent> = with_app(|a| {
        done.iter()
            .filter_map(|(id, at)| {
                let (title, author) = l.items.iter().find(|i| &i.id == id).map(|i| (i.title.clone(), i.author.clone()))
                    .or_else(|| find_book(a, id).map(|b| (b.title, b.author)))?;
                let when = local_day(*at).map(|d| d.format("%b %-d").to_string()).unwrap_or_default();
                Some(Recent { id: id.clone().into(), cover: a.covers.get(id).unwrap_or_default(), tint: tint(&title), title: title.into(), author: author.into(), when: when.into() })
            })
            .take(5)
            .collect()
    });
    for r in &recent {
        want_cover(&r.id);
    }
    with_app(|a| a.finished_model.set_vec(recent));
}

fn refresh_library() {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    let filter = [Filter::All, Filter::InProgress, Filter::NotStarted, Filter::Finished, Filter::Downloaded][st.get_filter().clamp(0, 4) as usize];
    let sort = [Sort::Title, Sort::Author, Sort::Added, Sort::Longest][st.get_sort().clamp(0, 3) as usize];
    let query = st.get_query().to_string();
    let count = with_app(|a| {
        a.covers.drop_queued();
        let books: Vec<Book> = a.books.iter().filter(|b| shown(a, b)).cloned().collect();
        let total = books.len();
        let shown: Vec<Book> = library::visible(&books, &a.progress, &a.downloaded, &query, filter, sort).into_iter().cloned().collect();
        let rows: Vec<BookCard> = shown.iter().map(|b| card(a, b)).collect();
        a.library.set_vec(rows);
        (shown.len(), total)
    });
    st.set_library_count(if count.0 == count.1 { format!("{} books", count.1) } else { format!("{} of {} books", count.0, count.1) }.into());
    covers_needed("library", 0, 29);
}

fn refresh_home() {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    let (hero, ids) = with_app(|a| {
        let in_progress: Vec<Book> = a.in_progress.iter().filter(|b| shown(a, b)).cloned().collect();
        let hero = in_progress.first().map(|b| (card(a, b), b.duration));
        let cont: Vec<BookCard> = in_progress.iter().skip(1).take(12).map(|b| card(a, b)).collect();
        let mut recent: Vec<Book> = a.books.iter().filter(|b| shown(a, b)).cloned().collect();
        recent.sort_by(|x, y| y.added.total_cmp(&x.added));
        let recent: Vec<BookCard> = recent.iter().take(12).map(|b| card(a, b)).collect();
        let ids: Vec<String> = hero.iter().map(|h| h.0.id.to_string()).chain(cont.iter().chain(&recent).map(|c| c.id.to_string())).collect();
        a.continuing.set_vec(cont);
        a.recent.set_vec(recent);
        (hero, ids)
    });
    st.set_has_hero(hero.is_some());
    if let Some((h, duration)) = hero {
        let done = duration * h.progress as f64;
        st.set_hero_listened(format!("{} listened", library::length(done)).into());
        st.set_hero_left(format!("{} left", library::length(duration - done)).into());
        st.set_hero(h);
    }
    for id in ids.iter().rev() {
        want_cover(id);
    }
}

/// Fetches the libraries, books, progress and books in progress, then fills Home and the Library.
fn load_library(client: Client, username: String) {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    st.set_busy(true);
    background(
        move || {
            let r = client.books().map(|books| {
                let progress = client.progress().unwrap_or_default();
                let in_progress = client.in_progress().unwrap_or_default();
                let libs = client.libraries().unwrap_or_default();
                Offline { books, in_progress, progress, libs }
            });
            // a sign-in saved before the username was kept: ask the server
            let username = if username.is_empty() { client.username().unwrap_or_default() } else { username };
            (client, r, username)
        },
        move |(client, r, username)| {
            let Some(ui) = window() else { return };
            let st = ui.global::<State>();
            st.set_busy(false);
            let saved = load_json::<Offline>("library.json").filter(|o| !o.books.is_empty());
            let (lib, offline) = match (r, saved) {
                (Ok(lib), _) => {
                    save_json("library.json", &lib);
                    (lib, false)
                }
                // no connection, but the library was loaded before: show it, and play what is downloaded
                (Err(e), Some(saved)) if !e.starts_with("Wrong username") => (saved, true),
                (Err(e), _) => {
                    st.set_message(e.into());
                    if !st.get_signed_in() {
                        st.set_route("settings".into());
                    }
                    return;
                }
            };
            st.set_offline(offline);
            st.set_message(SharedString::new());
            st.set_signed_in(true);
            st.set_signed_in_as(format!("Signed in to {}", client.base).into());
            st.set_username(username.clone().into());
            st.set_user_initial(username.chars().next().map_or("?".to_string(), |c| c.to_uppercase().to_string()).into());
            st.set_server_address(client.base.clone().into());
            save_session(Some((&client, &username)));
            with_app(|a| {
                a.dl.set_client((!offline).then(|| client.clone())); // downloads wait for a connection
                (a.client, a.books, a.progress, a.in_progress, a.libs) = (Some(client), lib.books, lib.progress, lib.in_progress, lib.libs);
            });
            show_libraries();
            refresh_home();
            refresh_library();
            if st.get_route() == "settings" {
                st.set_route("home".into());
            }
        },
    );
}

// ---- libraries shown and hidden (Settings > Libraries) ----
/// A book shows on Home, in the Library and in Series unless its library is hidden.
fn shown(a: &App, b: &Book) -> bool {
    b.library_id.is_empty() || !a.prefs.hidden_libs.contains(&b.library_id)
}

fn show_libraries() {
    let Some(ui) = window() else { return };
    let rows: Vec<LibRow> = with_app(|a| {
        let on = a.libs.iter().filter(|(id, _)| !a.prefs.hidden_libs.contains(id)).count();
        a.libs.iter()
            .map(|(id, name)| {
                let visible = !a.prefs.hidden_libs.contains(id);
                LibRow {
                    id: id.clone().into(),
                    name: name.clone().into(),
                    on: visible,
                    locked: visible && on < 2, // at least one library stays on
                    note: if !visible { "Hidden".into() } else if on < 2 { "Shown in the sidebar and on Home. At least one library stays on".into() } else { "Shown in the sidebar and on Home".into() },
                }
            })
            .collect()
    });
    ui.global::<State>().set_libraries(ModelRc::new(VecModel::from(rows)));
}

fn toggle_library(id: String) {
    with_app(|a| {
        if let Some(i) = a.prefs.hidden_libs.iter().position(|x| *x == id) {
            a.prefs.hidden_libs.remove(i);
        } else if a.libs.iter().filter(|(l, _)| !a.prefs.hidden_libs.contains(l)).count() > 1 {
            a.prefs.hidden_libs.push(id);
        }
    });
    save_prefs();
    show_libraries();
    refresh_home();
    refresh_library();
    if with_app(|a| !a.series.is_empty()) {
        fill_series();
    }
}

// ---- downloads ----
fn refresh_downloads() {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    let (items, bytes, limit, paused, folder) = with_app(|a| (a.dl.list(), a.dl.bytes(), a.dl.limit(), a.dl.paused(), a.dl.folder()));
    let gb = |b: u64| format!("{:.1} GB", b as f64 / 1e9);
    let rows: Vec<DlRow> = items.iter()
        .map(|i| {
            let status = match i.status {
                downloads::Status::Done => "Downloaded".to_string(),
                downloads::Status::Active => format!("Downloading {}%", (i.fraction() * 100.0) as u32),
                downloads::Status::Queued => if paused { "Paused".into() } else if st.get_offline() { "Waiting for a connection".into() } else { "Queued".into() },
                downloads::Status::Error => i.error.clone(),
            };
            DlRow {
                id: i.id.clone().into(), title: i.title.clone().into(), author: i.author.clone().into(), status: status.into(),
                progress: i.fraction(), done: i.status == downloads::Status::Done, failed: i.status == downloads::Status::Error,
                cover: with_app(|a| a.covers.get(&i.id)).unwrap_or_default(), tint: tint(&i.title),
            }
        })
        .collect();
    for r in &rows {
        want_cover(&r.id);
    }
    let book = st.get_book().id.to_string();
    match items.iter().find(|i| i.id == book) {
        None => st.set_book_dl(SharedString::new()),
        Some(i) => {
            let row = rows.iter().find(|r| r.id == book).unwrap();
            st.set_book_dl(match i.status {
                downloads::Status::Done => "done",
                downloads::Status::Active => "active",
                downloads::Status::Queued => "queued",
                downloads::Status::Error => "error",
            }.into());
            st.set_book_dl_text(row.status.clone());
        }
    }
    let done = items.iter().filter(|i| i.status == downloads::Status::Done).count();
    st.set_dl_any_done(done > 0);
    st.set_dl_paused(paused);
    st.set_dl_summary(format!("{} {} · {} on disk{}", items.len(), if items.len() == 1 { "book" } else { "books" }, gb(bytes),
        if limit > 0 { format!(" of {}", gb(limit)) } else { String::new() }).into());
    st.set_dl_folder(folder.to_string_lossy().to_string().into());
    st.set_dl_custom_folder(with_app(|a| a.prefs.folder.is_some()));
    st.set_dl_used(gb(bytes).into());
    st.set_downloads(ModelRc::new(VecModel::from(rows)));
    // the Library's Downloaded filter follows finished downloads
    let now: HashSet<String> = items.iter().filter(|i| i.status == downloads::Status::Done).map(|i| i.id.clone()).collect();
    if with_app(|a| std::mem::replace(&mut a.downloaded, now.clone()) != now) && st.get_filter() == 4 {
        refresh_library();
    }
}

fn choose_folder() {
    std::thread::spawn(|| {
        let Some(dir) = rfd::FileDialog::new().set_title("Where should downloads go?").pick_folder() else { return };
        let _ = slint::invoke_from_event_loop(move || {
            with_app(|a| {
                a.dl.set_folder(dir.clone());
                a.prefs.folder = Some(dir);
            });
            save_prefs();
            refresh_downloads();
        });
    });
}

fn open_book(id: String) {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    let route = st.get_route();
    if route != "book" && route != "player" {
        st.set_back_route(route);
    }
    st.set_route("book".into());
    st.set_book_loading(true);
    let Some(client) = with_app(|a| a.client.clone()) else { return };
    background({ let id = id.clone(); move || client.details(&id) }, move |r| {
        let Some(ui) = window() else { return };
        let st = ui.global::<State>();
        st.set_book_loading(false);
        match r {
            Ok(d) => {
                st.set_book(book_page(&id, &d));
                refresh_downloads(); // its Download button
                want_cover(&id);
            }
            Err(e) => st.set_message(e.into()),
        }
    });
}

fn book_page(id: &str, d: &Details) -> BookPage {
    let facts: Vec<Fact> = [
        ("Narrated by", d.narrators.clone()),
        ("Length", if d.duration > 0.0 { library::length(d.duration) } else { String::new() }),
        ("Chapters", if d.chapters > 0 { d.chapters.to_string() } else { String::new() }),
        ("Published", d.published.clone()),
        ("Language", d.language.clone()),
        ("Genres", d.genres.clone()),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(k, v)| Fact { k: k.into(), v: v.into() })
    .collect();
    BookPage {
        id: id.into(),
        title: d.title.clone().into(),
        subtitle: d.subtitle.clone().into(),
        by: d.authors.clone().into(),
        series: d.series.clone().into(),
        progress: d.progress as f32,
        progress_text: format!("{}% listened", (d.progress * 100.0).round()).into(),
        finished: d.finished,
        description: d.description.clone().into(),
        cover: with_app(|a| a.covers.get(id)).unwrap_or_default(),
        tint: tint(&d.title),
        facts: ModelRc::new(VecModel::from(facts)),
    }
}

fn set_finished(id: String, finished: bool) {
    let Some(client) = with_app(|a| a.client.clone()) else { return };
    background({ let (c, id) = (client.clone(), id.clone()); move || c.set_finished(&id, finished) }, move |r| {
        if let Err(e) = r {
            return say(format!("Couldn't update the book: {e}"));
        }
        with_app(|a| {
            a.progress.insert(id.clone(), if finished { 1.0 } else { 0.0 });
        });
        refresh_home();
        refresh_library();
        open_book(id);
    });
}

// ---- playback: one set of actions for the screens, the tray menu and the media keys ----
fn toggle() {
    let paused = with_app(|a| {
        if a.out.is_paused() {
            // back 10 seconds after a pause of more than a minute, so the listener picks up the thread
            if a.prefs.rewind && a.paused_at.is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
                if let Some(p) = &a.playing {
                    p.engine.seek((p.engine.status.pos() - 10.0).max(0.0));
                }
            }
            a.paused_at = None;
            a.out.play();
        } else {
            a.out.pause();
            a.paused_at = Some(Instant::now());
            // a pause ends the sleep timer, like the Tauri app; the default one starts again with the next play
            if let Some(p) = a.playing.as_mut() {
                p.sleep = Sleep::Off;
            }
        }
        (a.out.is_paused(), if a.out.is_paused() { None } else { start_default_sleep(a) })
    });
    if let Some(ui) = window() {
        let st = ui.global::<State>();
        if paused.0 {
            st.set_sleep_index(0);
        } else if let Some(i) = paused.1 {
            st.set_sleep_index(i);
        }
    }
    if paused.0 {
        report(false);
    }
}

/// Up and Down: volume in steps of 5%.
fn volume_step(d: f32) {
    with_app(|a| {
        a.prefs.volume = (a.prefs.volume + d).clamp(0.0, 1.0);
        a.out.set_volume(a.prefs.volume);
    });
    save_prefs();
}

fn set_paused(paused: bool) {
    if with_app(|a| a.playing.is_some() && a.out.is_paused() != paused) {
        toggle();
    }
}

fn skip(d: f64) {
    with_app(|a| if let Some(p) = &a.playing { p.engine.seek((p.engine.status.pos() + d).min(p.duration)) });
}

fn seek(t: f64) {
    with_app(|a| if let Some(p) = &a.playing { p.engine.seek(t) });
}

/// Next (1) or Previous (-1) chapter; a book without chapters skips forward 30 or back 15 instead.
fn chapter_step(dir: i32) {
    let target = with_app(|a| {
        let p = a.playing.as_ref()?;
        let t = p.engine.status.pos();
        if p.chapters.is_empty() {
            return Some(if dir > 0 { t + a.prefs.fwd as f64 } else { t - a.prefs.back as f64 }.min(p.duration));
        }
        if dir > 0 { book::next_start(&p.chapters, t) } else { book::prev_start(&p.chapters, t) }
    });
    if let Some(t) = target {
        seek(t);
    }
}

fn media_event(e: MediaControlEvent) {
    match e {
        MediaControlEvent::Play => set_paused(false),
        MediaControlEvent::Pause | MediaControlEvent::Stop => set_paused(true),
        MediaControlEvent::Toggle => toggle(),
        MediaControlEvent::Next => chapter_step(1),
        MediaControlEvent::Previous => chapter_step(-1),
        MediaControlEvent::Seek(SeekDirection::Forward) => skip(with_app(|a| a.prefs.fwd) as f64),
        MediaControlEvent::Seek(SeekDirection::Backward) => skip(-(with_app(|a| a.prefs.back) as f64)),
        MediaControlEvent::SeekBy(SeekDirection::Forward, d) => skip(d.as_secs_f64()),
        MediaControlEvent::SeekBy(SeekDirection::Backward, d) => skip(-d.as_secs_f64()),
        MediaControlEvent::SetPosition(p) => seek(p.0.as_secs_f64()),
        _ => {}
    }
}

/// A speed picked on the player: it applies now and is remembered for this book.
fn set_speed(s: f64) {
    with_app(|a| {
        a.speed = s;
        if let Some(p) = &a.playing {
            p.engine.set_speed(s);
            a.prefs.speeds.insert(p.id.clone(), s);
        }
    });
    save_prefs();
    if let Some(ui) = window() {
        ui.global::<State>().set_speed(s as f32);
    }
}

/// Settings > Default speed: for books whose speed hasn't been changed on the player, including the one playing.
fn set_default_speed(s: f64) {
    let follow = with_app(|a| {
        a.prefs.speed = s;
        a.playing.as_ref().is_some_and(|p| !a.prefs.speeds.contains_key(&p.id))
    });
    save_prefs();
    if follow {
        set_speed_now(s);
    }
    if let Some(ui) = window() {
        ui.global::<State>().set_default_speed(s as f32);
    }
}

/// Changes the playing speed without remembering it for the book.
fn set_speed_now(s: f64) {
    with_app(|a| {
        a.speed = s;
        if let Some(p) = &a.playing {
            p.engine.set_speed(s);
        }
    });
    if let Some(ui) = window() {
        ui.global::<State>().set_speed(s as f32);
    }
}

/// The default sleep timer (Settings), started when play is pressed. Returns the player's sleep menu index for it.
fn start_default_sleep(a: &mut App) -> Option<i32> {
    let p = a.playing.as_mut()?;
    if p.sleep != Sleep::Off || a.prefs.sleep == 0 {
        return None;
    }
    p.sleep = match a.prefs.sleep {
        1..=3 => Sleep::At(now_s() + SLEEPS[a.prefs.sleep as usize - 1] * 60.0),
        _ => book::chapter_at(&p.chapters, p.engine.status.pos()).map_or(Sleep::Off, Sleep::ChapterEnd),
    };
    Some(if a.prefs.sleep == 4 { 5 } else { a.prefs.sleep })
}

/// Remembers the place on this computer, and tells the server in the background when there is a session; `close` ends it.
/// Returns the thread so the caller can wait on exit.
fn report(close: bool) -> Option<std::thread::JoinHandle<()>> {
    with_app(|a| {
        let p = a.playing.as_mut()?;
        let (pos, listened) = (p.engine.status.pos(), p.listened);
        p.listened = 0.0;
        a.places.insert(p.id.clone(), pos);
        save_json("places.json", &a.places);
        if !a.prefs.sync {
            return None; // Settings > Sync progress is off
        }
        let (c, session) = (a.client.clone()?, p.session.clone()?);
        Some(std::thread::spawn(move || {
            if let Err(e) = c.sync(&session, pos, listened, close) {
                eprintln!("progress sync failed: {e}");
            }
        }))
    })
}

fn stop_book() -> Option<std::thread::JoinHandle<()>> {
    let sent = report(true);
    with_app(|a| {
        a.out.clear();
        a.playing = None;
    });
    sent
}

/// Plays a book, or pauses and resumes it when it is the one already loaded.
fn play_book(id: String) {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    if st.get_has_book() && st.get_now_id() == id {
        return toggle();
    }
    let Some((client, book, local, place)) = with_app(|a| Some((a.client.clone()?, find_book(a, &id)?, a.dl.local(&id), a.places.get(&id).copied()))) else { return };
    stop_book();
    st.set_message(format!("Opening {}…", book.title).into());
    background(
        {
            let id = book.id.clone();
            // A downloaded book plays from disk. It still opens a server session when it can, so the place and listening time are the
            // server's; without a connection it starts where this computer last left it. A book still plays when its bookmarks can't be read.
            move || {
                let online = client.play(&id);
                let s = match (online, local) {
                    (Ok(mut s), Some((tracks, duration, chapters))) => {
                        (s.tracks, s.duration, s.chapters) = (tracks, duration, chapters);
                        Ok(s)
                    }
                    (Ok(s), None) => Ok(s),
                    (Err(_), Some((tracks, duration, chapters))) => {
                        Ok(api::Session { id: String::new(), start: place.unwrap_or(0.0), duration, tracks, chapters })
                    }
                    (Err(e), None) => Err(e),
                };
                s.map(|s| {
                    let b = if s.id.is_empty() { vec![] } else { client.bookmarks(&id).unwrap_or_default() };
                    (client, s, b)
                })
            }
        },
        move |r| {
            let Some(ui) = window() else { return };
            let st = ui.global::<State>();
            let (client, mut s, bookmarks) = match r {
                Ok(v) => v,
                Err(e) => return st.set_message(e.into()),
            };
            st.set_message(SharedString::new());
            // a speed picked for this book wins over the default; with sync off, the book starts where this computer left it
            let (speed, place, sync) = with_app(|a| (a.prefs.speeds.get(&book.id).copied().unwrap_or(a.prefs.speed), a.places.get(&book.id).copied(), a.prefs.sync));
            if !sync {
                if let Some(t) = place {
                    s.start = t;
                }
            }
            with_app(|a| a.speed = speed);
            st.set_speed(speed as f32);
            let (http, token) = client.http();
            let engine = Engine::spawn(http, token, s.tracks, s.start, speed, with_app(|a| a.gains.clone()), |out| {
                let _ = slint::invoke_from_event_loop(move || {
                    with_app(|a| {
                        a.out.append(out);
                        a.out.play();
                    })
                });
            });
            st.set_now_id(book.id.clone().into());
            st.set_now_title(book.title.clone().into());
            st.set_now_author(book.author.clone().into());
            st.set_now_tint(tint(&book.title));
            st.set_now_cover(with_app(|a| a.covers.get(&book.id)).unwrap_or_default());
            want_cover(&book.id);
            st.set_has_book(true);
            st.set_has_chapters(!s.chapters.is_empty());
            st.set_sleep_index(0);
            show_bookmarks(&st, &bookmarks);
            let cover = client.cover_url(&book.id);
            with_app(|a| {
                if let Some(m) = a.media.as_mut() {
                    media::show_book(m, &book.title, &book.author, &cover, s.duration);
                }
                a.shown.0 = None;
                a.playing = Some(Playing {
                    engine, session: (!s.id.is_empty()).then_some(s.id), id: book.id, ended: false, title: book.title, duration: s.duration,
                    listened: 0.0, last_tick: Instant::now(),
                    chapters: s.chapters, bookmarks, shown_chapter: None, sleep: Sleep::Off,
                });
            });
            if let Some(i) = with_app(start_default_sleep) {
                st.set_sleep_index(i);
            }
        },
    );
}

// four times a second: move the position bar, show errors and the end of the book, keep the OS overlay and the tray in step,
// and report progress every 15 seconds heard
fn tick(st: &State, tray: &Tray) {
    let due = with_app(|a| {
        let paused = a.out.is_paused();
        let Some(p) = a.playing.as_mut() else { return false };
        let state = p.engine.status.state.load(Relaxed);
        let playing = !paused && state == player::PLAYING;
        // the overlay moves its own timeline while playing, so it only needs the state on a change and a correction now and then
        if a.shown.0 != Some(playing) || a.shown.1.elapsed() > Duration::from_secs(5) {
            if let Some(m) = a.media.as_mut() {
                media::show_state(m, Some(playing), p.engine.status.pos());
            }
            tray.set_playing(playing);
            tray.set_has_book(true);
            a.shown = (Some(playing), Instant::now());
        }
        let now = Instant::now();
        if playing {
            p.listened += (now - p.last_tick).as_secs_f64();
        }
        p.last_tick = now;
        let pos = p.engine.status.pos();

        // the sleep timer: pause, and at a chapter's end also park at that end so Play goes on with the next chapter
        let mut slept = false;
        if !paused && p.sleep.due(now_s(), &p.chapters, pos) {
            a.out.pause();
            if let Sleep::ChapterEnd(n) = p.sleep {
                p.engine.seek(p.chapters[n].end);
            }
            p.sleep = Sleep::Off;
            st.set_sleep_index(0);
            slept = true;
        }
        // with "play the next chapter automatically" off, stop at each chapter's end and park there, so Play goes on with the next
        if !paused && !slept && !a.prefs.auto_next {
            if let Some(i) = book::chapter_at(&p.chapters, pos).filter(|&i| Sleep::ChapterEnd(i).due(0.0, &p.chapters, pos)) {
                a.out.pause();
                p.engine.seek(p.chapters[i].end);
                slept = true;
            }
        }
        if slept {
            a.paused_at = Some(Instant::now());
        }
        // the last 30 s of a sleep timer fade out; full volume again once it has paused, is turned off or is extended
        a.out.set_volume(a.prefs.volume * book::fade(p.sleep.left(now_s(), &p.chapters, pos, a.speed)));
        st.set_sleep_text(match p.sleep {
            Sleep::Off => SharedString::new(),
            Sleep::At(at) => format!("Sleeping in {}", clock(at - now_s())).into(),
            Sleep::ChapterEnd(_) => "Sleeping at the end of this chapter".into(),
        });

        // the seek bars and times follow the current chapter
        let ch = book::chapter_at(&p.chapters, pos);
        let (start, end) = ch.map_or((0.0, p.duration), |i| (p.chapters[i].start, p.chapters[i].end));
        if p.shown_chapter != Some(ch) {
            p.shown_chapter = Some(ch);
            st.set_chapter_text(ch.map_or(SharedString::new(), |i| format!("{} · {} of {}", p.chapters[i].title, i + 1, p.chapters.len()).into()));
            st.set_seek_min(start as f32);
            st.set_seek_max(end as f32);
            let rows: Vec<ListRow> = p.chapters.iter().enumerate()
                .map(|(i, c)| ListRow { title: c.title.clone().into(), sub: clock(c.start).into(), current: Some(i) == ch })
                .collect();
            st.set_chapters(ModelRc::new(VecModel::from(rows)));
        }
        if !st.get_seeking() {
            st.set_position(pos as f32);
        }
        st.set_pos_text(clock(pos - start).into());
        st.set_left_text(format!("-{}", clock(end - pos)).into());
        st.set_book_left(format!("{} left in the book", library::length(p.duration - pos)).into());
        st.set_playing(playing && !slept);
        match state {
            player::FAILED => st.set_message(p.engine.status.error.lock().unwrap().clone().into()),
            player::ENDED if !p.ended => {
                p.ended = true;
                st.set_message(format!("Finished {}", p.title).into());
                // finished books free their space, when the listener asked for that
                if a.prefs.auto_remove && a.dl.get(&p.id).is_some() {
                    a.dl.remove(&p.id);
                }
            }
            _ => {}
        }
        p.listened >= 15.0 || slept
    });
    if due {
        report(false);
    }
}

fn show_bookmarks(st: &State, b: &[Bookmark]) {
    let rows: Vec<ListRow> = b.iter().map(|b| ListRow { title: b.title.clone().into(), sub: clock(b.time).into(), current: false }).collect();
    st.set_bookmarks(ModelRc::new(VecModel::from(rows)));
}

fn set_sleep(choice: i32) {
    with_app(|a| {
        let Some(p) = a.playing.as_mut() else { return };
        p.sleep = match choice {
            1..=4 => Sleep::At(now_s() + choice as f64 * 15.0 * 60.0),
            5 => book::chapter_at(&p.chapters, p.engine.status.pos()).map_or(Sleep::Off, Sleep::ChapterEnd),
            _ => Sleep::Off,
        };
    });
}

fn extend_sleep() {
    with_app(|a| {
        let speed = a.speed;
        let Some(p) = a.playing.as_mut() else { return };
        p.sleep = p.sleep.extended(now_s(), &p.chapters, p.engine.status.pos(), speed);
    });
}

fn add_bookmark() {
    let Some((client, id, b)) = with_app(|a| {
        let p = a.playing.as_ref()?;
        let t = p.engine.status.pos().floor();
        Some((a.client.clone()?, p.id.clone(), Bookmark { time: t, title: book::bookmark_title(&p.chapters, &p.title, t, clock) }))
    }) else { return };
    background({ let b = b.clone(); move || client.add_bookmark(&id, &b) }, move |r| {
        if let Err(e) = r {
            return say(format!("Couldn't save the bookmark: {e}"));
        }
        let Some(ui) = window() else { return };
        with_app(|a| if let Some(p) = a.playing.as_mut() {
            p.bookmarks.retain(|x| x.time != b.time); // the server keeps one bookmark per second
            p.bookmarks.push(b);
            p.bookmarks.sort_by(|x, y| x.time.total_cmp(&y.time));
            show_bookmarks(&ui.global::<State>(), &p.bookmarks);
        });
    });
}

fn remove_bookmark(i: usize) {
    let Some((client, id, time)) = with_app(|a| {
        let p = a.playing.as_ref()?;
        Some((a.client.clone()?, p.id.clone(), p.bookmarks.get(i)?.time))
    }) else { return };
    background(move || client.remove_bookmark(&id, time), move |r| {
        if let Err(e) = r {
            return say(format!("Couldn't remove the bookmark: {e}"));
        }
        let Some(ui) = window() else { return };
        with_app(|a| if let Some(p) = a.playing.as_mut() {
            p.bookmarks.retain(|x| x.time != time);
            show_bookmarks(&ui.global::<State>(), &p.bookmarks);
        });
    });
}

// ---- updates ----
fn upd_status(busy: bool, status: &str, available: &str) {
    if let Some(ui) = window() {
        let st = ui.global::<State>();
        st.set_upd_busy(busy);
        st.set_upd_status(status.into());
        st.set_upd_available(available.into());
    }
}

/// Quiet is the check at launch: it only speaks up when there is an update, and leaves errors (no token, offline) unsaid.
fn check_updates(quiet: bool) {
    let token = with_app(|a| a.prefs.gh_token.clone());
    if quiet && token.is_empty() {
        return;
    }
    if !quiet {
        upd_status(true, "Checking…", "");
    }
    background(move || updater::check(&token), move |r| match r {
        Ok(Some(u)) => {
            let (v, notes) = (u.version.clone(), u.body.clone().unwrap_or_default());
            with_app(|a| a.update = Some(std::sync::Arc::new(u)));
            upd_status(false, &if notes.is_empty() { format!("Version {v} is available") } else { format!("Version {v} is available
{notes}") }, &v);
        }
        Ok(None) if !quiet => upd_status(false, "You have the latest version", ""),
        Err(e) if !quiet => upd_status(false, &e, ""),
        _ => {}
    });
}

/// Saves the listening place, then downloads, checks and installs the update. The installer replaces the app; on a Mac the new copy
/// is opened before this one quits.
fn install_update() {
    let Some(u) = with_app(|a| a.update.clone()) else { return };
    if let Some(t) = stop_book() {
        let _ = t.join();
    }
    upd_status(true, "Downloading the update…", &u.version);
    background(move || updater::install(&u).map(|_| u), |r| match r {
        Ok(_) => {
            if cfg!(target_os = "macos") {
                // …/Lectern.app/Contents/MacOS/lectern → …/Lectern.app
                if let Some(app) = std::env::current_exe().ok().and_then(|p| p.ancestors().nth(3).map(|a| a.to_path_buf())) {
                    let _ = std::process::Command::new("open").arg("-n").arg(app).spawn();
                }
            }
            let _ = slint::quit_event_loop();
        }
        Err(e) => upd_status(false, &e, ""),
    });
}

// ---- the tray mini player ----
#[derive(Default)]
struct MiniState {
    shown_at: Option<Instant>,
    hidden_at: Option<Instant>,
    styled: bool, // taken out of the taskbar (done once, the first time it opens)
}

thread_local! {
    static MINI: RefCell<Option<slint::Weak<MiniPlayer>>> = const { RefCell::new(None) };
}

fn hide_mini(mini: &MiniPlayer, ms: &RefCell<MiniState>) {
    let _ = mini.hide();
    ms.borrow_mut().hidden_at = Some(Instant::now());
}

/// Copies what the main window shows about the playing book into the mini player.
fn fill_mini(mini: &MiniPlayer) {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    mini.set_has_book(st.get_has_book());
    mini.set_book_title(st.get_now_title());
    mini.set_chapter(st.get_chapter_text());
    mini.set_author(st.get_now_author());
    mini.set_cover(st.get_now_cover());
    mini.set_tint(st.get_now_tint());
    mini.set_playing(st.get_playing());
    mini.set_has_chapters(st.get_has_chapters());
    mini.set_back_secs(st.get_back_secs());
    mini.set_fwd_secs(st.get_fwd_secs());
    let span = (st.get_seek_max() - st.get_seek_min()).max(1.0);
    mini.set_progress(((st.get_position() - st.get_seek_min()) / span).clamp(0.0, 1.0));
    mini.set_t0(st.get_pos_text());
    mini.set_t1(st.get_left_text());
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sink = rodio::DeviceSinkBuilder::open_default_sink()?;
    sink.log_on_drop(false);
    let out = rodio::Player::connect_new(sink.mixer());
    let prefs: Prefs = load_json("prefs.json").unwrap_or_default();

    let ui = AppWindow::new()?;
    let covers = Covers::new(Box::new(|id, px| {
        let _ = slint::invoke_from_event_loop(move || cover_arrived(id, px));
    }));
    let dl = Downloads::new(
        config_dir().unwrap_or_default().join("downloads.json"),
        prefs.folder.clone().unwrap_or_else(default_folder),
        prefs.limit_gb * 1_000_000_000,
        8 * 1024 * 1024,
        || {
            let _ = slint::invoke_from_event_loop(refresh_downloads);
        },
        || METERED_OK.load(Relaxed) || !metered::is_metered(),
    );
    METERED_OK.store(prefs.metered, Relaxed);
    let (library, continuing, recent) = (Rc::new(VecModel::default()), Rc::new(VecModel::default()), Rc::new(VecModel::default()));
    let (series_model, series_books, finished_model) = (Rc::new(VecModel::default()), Rc::new(VecModel::default()), Rc::new(VecModel::default()));
    let gains = Arc::new(eq::Gains::default());
    gains.set(prefs.eq);
    let eq_model = Rc::new(VecModel::from(prefs.eq.to_vec()));
    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            ui: ui.as_weak(), _sink: sink, out, client: None, books: vec![], in_progress: vec![], progress: HashMap::new(),
            library: library.clone(), continuing: continuing.clone(), recent: recent.clone(), covers, playing: None, speed: prefs.speed,
            series: vec![], series_model: series_model.clone(), series_books: series_books.clone(), finished_model: finished_model.clone(), listening: None,
            media: None, shown: (None, Instant::now()), dl, downloaded: HashSet::new(), prefs: prefs.clone(),
            places: load_json("places.json").unwrap_or_default(), gains: gains.clone(), eq_model: eq_model.clone(), paused_at: None, update: None, libs: vec![],
        })
    });

    if let Some(i) = app_icon() {
        ui.set_app_icon(i);
    }
    let theme = ui.global::<Theme>();
    theme.set_dark(prefs.dark);
    if !cfg!(windows) {
        theme.set_sans("".into()); // the system font
    }
    let st = ui.global::<State>();
    st.set_library(library.into());
    st.set_continue_books(continuing.into());
    st.set_recent_books(recent.into());
    st.set_series_list(series_model.into());
    st.set_series_books(series_books.into());
    st.set_recent_finished(finished_model.into());
    st.set_speed(prefs.speed as f32);
    st.set_dl_limit_index(LIMITS_GB.iter().position(|g| *g == prefs.limit_gb).unwrap_or(2) as i32);
    st.set_auto_remove(prefs.auto_remove);
    st.set_back_secs(prefs.back as i32);
    st.set_fwd_secs(prefs.fwd as i32);
    st.set_back_index(BACKS.iter().position(|s| *s == prefs.back).unwrap_or(1) as i32);
    st.set_fwd_index(FWDS.iter().position(|s| *s == prefs.fwd).unwrap_or(1) as i32);
    st.set_rewind(prefs.rewind);
    st.set_auto_next(prefs.auto_next);
    st.set_eq(eq_model.clone().into());
    st.set_eq_preset(eq_preset(prefs.eq));
    st.on_set_skips(|b, f| {
        let (back, fwd) = (BACKS[b.clamp(0, 2) as usize], FWDS[f.clamp(0, 2) as usize]);
        with_app(|a| (a.prefs.back, a.prefs.fwd) = (back, fwd));
        save_prefs();
        if let Some(ui) = window() {
            let st = ui.global::<State>();
            st.set_back_secs(back as i32);
            st.set_fwd_secs(fwd as i32);
        }
    });
    st.on_set_rewind(|on| {
        with_app(|a| a.prefs.rewind = on);
        save_prefs();
    });
    st.on_set_auto_next(|on| {
        with_app(|a| a.prefs.auto_next = on);
        save_prefs();
    });
    st.on_set_eq_preset(|i| {
        let Some((_, db)) = eq::PRESETS.get(i as usize) else { return }; // "Custom" keeps the sliders as they are
        set_eq(*db, true);
    });
    st.on_set_eq_band(|i, v| {
        let mut db = with_app(|a| a.prefs.eq);
        db[i.clamp(0, 4) as usize] = v;
        set_eq(db, false);
    });
    refresh_downloads();

    st.on_download(|id| {
        with_app(|a| if let Some(b) = find_book(a, &id) { a.dl.add(&b.id, &b.title, &b.author) });
    });
    st.on_remove_download(|id| with_app(|a| a.dl.remove(&id)));
    st.on_retry_download(|id| with_app(|a| a.dl.retry(&id)));
    st.on_pause_downloads(|on| with_app(|a| a.dl.pause(on)));
    st.on_remove_finished(|| with_app(|a| a.dl.remove_finished()));
    st.on_choose_folder(choose_folder);
    st.on_set_limit(|i| {
        let gb = LIMITS_GB[i.clamp(0, 4) as usize];
        with_app(|a| {
            a.prefs.limit_gb = gb;
            a.dl.set_limit(gb * 1_000_000_000);
        });
        save_prefs();
    });
    st.on_set_auto_remove(|on| {
        with_app(|a| a.prefs.auto_remove = on);
        save_prefs();
    });

    st.on_sign_in(|server, user, pass| {
        say("Signing in…");
        if let Some(ui) = window() {
            ui.global::<State>().set_busy(true);
        }
        let (server, user, pass) = (server.to_string(), user.to_string(), pass.to_string());
        background(move || Client::login(&server, &user, &pass), |r| match r {
            Ok((c, user)) => {
                if let Some(ui) = window() {
                    ui.global::<State>().set_sign_in_error(SharedString::new());
                }
                load_library(c, user)
            }
            Err(e) => {
                if let Some(ui) = window() {
                    let st = ui.global::<State>();
                    st.set_busy(false);
                    st.set_sign_in_error(e.clone().into());
                }
                say(e);
            }
        });
    });
    st.on_sign_out(|| {
        stop_book();
        save_session(None);
        save_json("library.json", &Offline::default());
        with_app(|a| {
            a.dl.set_client(None);
            (a.client, a.books, a.in_progress) = (None, vec![], vec![]);
            a.progress.clear();
            for m in [&a.library, &a.continuing, &a.recent] {
                m.set_vec(vec![]);
            }
        });
        if let Some(ui) = window() {
            let st = ui.global::<State>();
            st.set_has_book(false);
            st.set_has_hero(false);
            st.set_signed_in(false);
            st.set_route("settings".into());
        }
    });
    st.on_toggle_theme(|| {
        if let Some(ui) = window() {
            let t = ui.global::<Theme>();
            t.set_dark(!t.get_dark());
            with_app(|a| a.prefs.dark = t.get_dark());
            save_prefs();
            MINI.with(|m| if let Some(mini) = m.borrow().as_ref().and_then(|w| w.upgrade()) { mini.global::<Theme>().set_dark(t.get_dark()) });
        }
    });
    st.on_refresh_library(refresh_library);
    st.on_covers_needed(|list, first, last| covers_needed(&list, first, last));
    st.on_open_book(|id| open_book(id.to_string()));
    st.on_play_book(|id| play_book(id.to_string()));
    st.on_set_finished(|id, f| set_finished(id.to_string(), f));
    st.on_show_series(show_series);
    st.on_open_series(|id| open_series(id.to_string()));
    st.on_show_stats(show_stats);
    st.set_version(updater::current().into());
    st.set_gh_token(prefs.gh_token.clone().into());
    st.on_save_token(|| {
        if let Some(ui) = window() {
            let t = ui.global::<State>().get_gh_token().trim().to_string();
            with_app(|a| a.prefs.gh_token = t);
            save_prefs();
        }
    });
    st.on_check_updates(|| check_updates(false));
    st.on_install_update(install_update);
    st.set_default_speed(prefs.speed as f32);
    st.set_sleep_default(prefs.sleep);
    st.set_tray(prefs.tray);
    st.set_sync(prefs.sync);
    st.set_metered(prefs.metered);
    with_app(|a| a.out.set_volume(prefs.volume));
    st.on_toggle_library(|id| toggle_library(id.to_string()));
    st.on_set_default_speed(|s| set_default_speed(s as f64));
    st.on_set_sleep_default(|i| {
        with_app(|a| a.prefs.sleep = i.clamp(0, 4));
        save_prefs();
    });
    st.on_set_tray(|on| {
        with_app(|a| a.prefs.tray = on);
        save_prefs();
    });
    st.on_set_sync(|on| {
        with_app(|a| a.prefs.sync = on);
        save_prefs();
    });
    st.on_set_metered(|on| {
        METERED_OK.store(on, Relaxed);
        with_app(|a| {
            a.prefs.metered = on;
            a.dl.pause(a.dl.paused()); // wakes the queue so it looks again now
        });
        save_prefs();
    });
    st.on_use_default_folder(|| {
        with_app(|a| {
            a.prefs.folder = None;
            a.dl.set_folder(default_folder());
        });
        save_prefs();
        refresh_downloads();
    });
    st.on_volume_step(|d| volume_step(d));

    st.on_toggle(toggle);
    st.on_skip(|d| skip(d as f64));
    st.on_seek(|t| seek(t as f64));
    st.on_set_speed(|s| set_speed(s as f64));
    st.on_chapter_step(chapter_step);
    st.on_pick_chapter(|i| {
        if let Some(t) = with_app(|a| Some(a.playing.as_ref()?.chapters.get(i as usize)?.start)) {
            seek(t);
        }
    });
    st.on_pick_bookmark(|i| {
        if let Some(t) = with_app(|a| Some(a.playing.as_ref()?.bookmarks.get(i as usize)?.time)) {
            seek(t);
        }
    });
    st.on_add_bookmark(add_bookmark);
    st.on_remove_bookmark(|i| remove_bookmark(i as usize));
    st.on_sleep(set_sleep);
    st.on_extend_sleep(extend_sleep);

    // closing the window only hides it (Slint's default) and playback goes on; the tray brings it back or quits
    let tray = Tray::new()?;
    match tray_icon() {
        Some(i) => tray.set_image(i),
        None => eprintln!("the tray icon could not be decoded"),
    }
    let w = ui.as_weak();
    tray.on_show_window(move || {
        if let Some(ui) = w.upgrade() {
            let _ = ui.show();
        }
    });
    tray.on_toggle(toggle);
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    tray.show()?;

    // the mini player: left-click on the tray icon opens it beside the icon, a click anywhere else closes it
    let mini = MiniPlayer::new()?;
    mini.global::<Theme>().set_dark(prefs.dark);
    if let Some(i) = app_icon() {
        mini.set_app_icon(i);
    }
    if !cfg!(windows) {
        mini.global::<Theme>().set_sans("".into());
    }
    let mini_state = Rc::new(RefCell::new(MiniState::default()));
    MINI.with(|m| *m.borrow_mut() = Some(mini.as_weak()));
    let (m, ms) = (mini.as_weak(), mini_state.clone());
    tray.on_show_mini(move || {
        let Some(mini) = m.upgrade() else { return };
        if mini.window().is_visible() {
            return hide_mini(&mini, &ms);
        }
        // the click on the icon is what took the focus away and closed it: don't reopen straight away
        if ms.borrow().hidden_at.is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
            return;
        }
        fill_mini(&mini);
        mini::place(mini.window(), 340.0, 268.0);
        let _ = mini.show();
        let first = !std::mem::replace(&mut ms.borrow_mut().styled, true);
        if first {
            // the taskbar button only goes once the window is shown again with the new style
            mini::hide_from_taskbar(mini.window());
            let _ = mini.hide();
            let _ = mini.show();
            mini::place(mini.window(), 340.0, 268.0);
        }
        mini::bring_to_front(mini.window());
        ms.borrow_mut().shown_at = Some(Instant::now());
    });
    mini.on_toggle(toggle);
    mini.on_skip(|d| skip(d as f64));
    mini.on_chapter_step(chapter_step);
    let (m, ms) = (mini.as_weak(), mini_state.clone());
    mini.on_dismiss(move || {
        if let Some(mini) = m.upgrade() {
            hide_mini(&mini, &ms);
        }
    });
    let (w, m, ms) = (ui.as_weak(), mini.as_weak(), mini_state.clone());
    mini.on_open_main(move || {
        if let Some(mini) = m.upgrade() {
            hide_mini(&mini, &ms);
        }
        if let Some(ui) = w.upgrade() {
            let _ = ui.show();
            mini::bring_to_front(ui.window());
        }
    });

    let timer = Timer::default();
    let (w, t, m, ms) = (ui.as_weak(), tray.as_weak(), mini.as_weak(), mini_state.clone());
    timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        if let (Some(ui), Some(tray)) = (w.upgrade(), t.upgrade()) {
            tick(&ui.global::<State>(), &tray);
        }
        if let Some(mini) = m.upgrade().filter(|m| m.window().is_visible()) {
            fill_mini(&mini);
            let settled = ms.borrow().shown_at.is_some_and(|t| t.elapsed() > Duration::from_millis(400));
            if settled && !mini::has_focus(mini.window()) {
                hide_mini(&mini, &ms);
            }
        }
    });

    ui.show()?;
    // closing the window hides it and Lectern keeps playing from the tray, unless that is turned off in Settings
    ui.window().on_close_requested(|| {
        if !with_app(|a| a.prefs.tray) {
            let _ = slint::quit_event_loop();
        }
        slint::CloseRequestResponse::HideWindow
    });
    covers::set_scale(ui.window().scale_factor()); // known once the window is on a screen, and needed before the first cover
    match load_session() {
        Some((c, user)) => {
            ui.set_server(c.base.clone().into());
            load_library(c, user);
        }
        None => st.set_route("settings".into()),
    }
    check_updates(true);
    // media keys need the window's handle, which exists once it is shown
    let controls = media::register(ui.window(), |e| {
        let _ = slint::invoke_from_event_loop(move || media_event(e));
    });
    with_app(|a| a.media = controls);
    slint::run_event_loop_until_quit()?;
    if let Some(t) = stop_book() {
        let _ = t.join(); // the closing progress report
    }
    Ok(())
}
