// Lectern, native: the Slint screens over the playback core. Network calls run on worker threads and hand their results back to the UI
// thread with invoke_from_event_loop; everything the UI owns lives in APP on that thread.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod api;
mod book;
mod covers;
mod http_source;
mod library;
mod media;
mod player;
mod stretch;

use api::{Book, Client, Details};
use book::{Bookmark, Chapter, Sleep};
use covers::Covers;
use library::{Filter, Sort};
use player::Engine;
use slint::{Color, ComponentHandle, Image, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use souvlaki::{MediaControlEvent, SeekDirection};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

slint::include_modules!();

struct Playing {
    engine: Engine,
    session: String,
    id: String,
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
    covers: Covers,
    playing: Option<Playing>,
    speed: f64,
    media: Option<souvlaki::MediaControls>,
    shown: (Option<bool>, Instant), // the play state last published to the OS overlay and the tray, and when
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

fn save_session(c: Option<&Client>) {
    let Some(d) = config_dir() else { return };
    let p = d.join("session.json");
    let _ = match c {
        Some(c) => std::fs::create_dir_all(&d).and_then(|_| std::fs::write(&p, serde_json::json!({ "server": c.base, "token": c.token }).to_string())),
        None => std::fs::remove_file(&p),
    };
}

fn load_session() -> Option<Client> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config_dir()?.join("session.json")).ok()?).ok()?;
    Client::new(v["server"].as_str()?, v["token"].as_str()?).ok()
}

fn save_prefs(dark: bool, speed: f64) {
    if let Some(d) = config_dir() {
        let _ = std::fs::create_dir_all(&d).and_then(|_| std::fs::write(d.join("prefs.json"), serde_json::json!({ "dark": dark, "speed": speed }).to_string()));
    }
}

fn load_prefs() -> (bool, f64) {
    let v: serde_json::Value = config_dir()
        .and_then(|d| std::fs::read_to_string(d.join("prefs.json")).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    (v["dark"].as_bool().unwrap_or(false), v["speed"].as_f64().unwrap_or(1.0))
}

/// The tray icon as RGBA pixels, decoded from the PNG built into the exe.
fn tray_icon() -> Option<Image> {
    let mut d = png::Decoder::new(std::io::Cursor::new(include_bytes!("../ui/icons/tray.png")));
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
            if let Some(mut r) = model.row_data(i).filter(|r| r.id == id) {
                r.cover = img.clone();
                model.set_row_data(i, r);
            }
        }
    };
    with_app(|a| {
        for m in [&a.library, &a.continuing, &a.recent] {
            set(m, &id, &img);
            for old in &evicted {
                set(m, old, &Image::default());
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
    if list != "library" || first < 0 {
        return;
    }
    let ids: Vec<String> = with_app(|a| (first..=last).filter_map(|i| a.library.row_data(i as usize)).map(|r| r.id.to_string()).collect());
    for id in ids.iter().rev() {
        want_cover(id); // newest request is served first, so the top of the screen is asked for last
    }
}

fn refresh_library() {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    let filter = [Filter::All, Filter::InProgress, Filter::NotStarted, Filter::Finished][st.get_filter().clamp(0, 3) as usize];
    let sort = [Sort::Title, Sort::Author, Sort::Added, Sort::Longest][st.get_sort().clamp(0, 3) as usize];
    let query = st.get_query().to_string();
    let count = with_app(|a| {
        a.covers.drop_queued();
        let shown: Vec<Book> = library::visible(&a.books, &a.progress, &query, filter, sort).into_iter().cloned().collect();
        let rows: Vec<BookCard> = shown.iter().map(|b| card(a, b)).collect();
        a.library.set_vec(rows);
        (shown.len(), a.books.len())
    });
    st.set_library_count(if count.0 == count.1 { format!("{} books", count.1) } else { format!("{} of {} books", count.0, count.1) }.into());
    covers_needed("library", 0, 29);
}

fn refresh_home() {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    let (hero, ids) = with_app(|a| {
        let in_progress = a.in_progress.clone();
        let hero = in_progress.first().map(|b| (card(a, b), b.duration));
        let cont: Vec<BookCard> = in_progress.iter().skip(1).take(12).map(|b| card(a, b)).collect();
        let mut recent: Vec<Book> = a.books.clone();
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

/// Fetches the books, progress and books in progress, then fills Home and the Library.
fn load_library(client: Client) {
    let Some(ui) = window() else { return };
    let st = ui.global::<State>();
    st.set_busy(true);
    background(
        move || {
            let books = client.books()?;
            let progress = client.progress().unwrap_or_default();
            let in_progress = client.in_progress().unwrap_or_default();
            Ok::<_, String>((client, books, progress, in_progress))
        },
        |r| {
            let Some(ui) = window() else { return };
            let st = ui.global::<State>();
            st.set_busy(false);
            match r {
                Ok((client, books, progress, in_progress)) => {
                    st.set_message(SharedString::new());
                    st.set_signed_in(true);
                    st.set_signed_in_as(format!("Signed in to {}", client.base).into());
                    save_session(Some(&client));
                    with_app(|a| (a.client, a.books, a.progress, a.in_progress) = (Some(client), books, progress, in_progress));
                    refresh_home();
                    refresh_library();
                    if st.get_route() == "settings" {
                        st.set_route("home".into());
                    }
                }
                Err(e) => {
                    st.set_message(e.into());
                    if !st.get_signed_in() {
                        st.set_route("settings".into());
                    }
                }
            }
        },
    );
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
        if a.out.is_paused() { a.out.play() } else { a.out.pause() }
        a.out.is_paused()
    });
    if paused {
        report(false);
    }
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
            return Some(if dir > 0 { t + 30.0 } else { t - 15.0 }.min(p.duration));
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
        MediaControlEvent::Seek(SeekDirection::Forward) => skip(30.0),
        MediaControlEvent::Seek(SeekDirection::Backward) => skip(-15.0),
        MediaControlEvent::SeekBy(SeekDirection::Forward, d) => skip(d.as_secs_f64()),
        MediaControlEvent::SeekBy(SeekDirection::Backward, d) => skip(-d.as_secs_f64()),
        MediaControlEvent::SetPosition(p) => seek(p.0.as_secs_f64()),
        _ => {}
    }
}

fn set_speed(s: f64) {
    with_app(|a| {
        a.speed = s;
        if let Some(p) = &a.playing {
            p.engine.set_speed(s);
        }
    });
    if let Some(ui) = window() {
        ui.global::<State>().set_speed(s as f32);
        save_prefs(ui.global::<Theme>().get_dark(), s);
    }
}

/// Tells the server where the listener is, in the background; `close` ends the session. Returns the thread so the caller can wait on exit.
fn report(close: bool) -> Option<std::thread::JoinHandle<()>> {
    with_app(|a| {
        let (c, p) = (a.client.clone()?, a.playing.as_mut()?);
        let (session, pos, listened) = (p.session.clone(), p.engine.status.pos(), p.listened);
        p.listened = 0.0;
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
    let Some((client, book)) = with_app(|a| Some((a.client.clone()?, find_book(a, &id)?))) else { return };
    stop_book();
    st.set_message(format!("Opening {}…", book.title).into());
    background(
        {
            let id = book.id.clone();
            // a book still plays when its bookmarks can't be read
            move || client.play(&id).map(|s| { let b = client.bookmarks(&id).unwrap_or_default(); (client, s, b) })
        },
        move |r| {
            let Some(ui) = window() else { return };
            let st = ui.global::<State>();
            let (client, s, bookmarks) = match r {
                Ok(v) => v,
                Err(e) => return st.set_message(e.into()),
            };
            st.set_message(SharedString::new());
            let speed = with_app(|a| a.speed);
            let (http, token) = client.http();
            let engine = Engine::spawn(http, token, s.tracks, s.start, speed, |out| {
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
                    engine, session: s.id, id: book.id, title: book.title, duration: s.duration, listened: 0.0, last_tick: Instant::now(),
                    chapters: s.chapters, bookmarks, shown_chapter: None, sleep: Sleep::Off,
                });
            });
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
            player::ENDED => st.set_message(format!("Finished {}", p.title).into()),
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sink = rodio::DeviceSinkBuilder::open_default_sink()?;
    sink.log_on_drop(false);
    let out = rodio::Player::connect_new(sink.mixer());
    let (dark, speed) = load_prefs();

    let ui = AppWindow::new()?;
    let covers = Covers::new(Box::new(|id, px| {
        let _ = slint::invoke_from_event_loop(move || cover_arrived(id, px));
    }));
    let (library, continuing, recent) = (Rc::new(VecModel::default()), Rc::new(VecModel::default()), Rc::new(VecModel::default()));
    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            ui: ui.as_weak(), _sink: sink, out, client: None, books: vec![], in_progress: vec![], progress: HashMap::new(),
            library: library.clone(), continuing: continuing.clone(), recent: recent.clone(), covers, playing: None, speed,
            media: None, shown: (None, Instant::now()),
        })
    });

    let theme = ui.global::<Theme>();
    theme.set_dark(dark);
    if !cfg!(windows) {
        theme.set_sans("".into()); // the system font
    }
    let st = ui.global::<State>();
    st.set_library(library.into());
    st.set_continue_books(continuing.into());
    st.set_recent_books(recent.into());
    st.set_speed(speed as f32);

    st.on_sign_in(|server, user, pass| {
        say("Signing in…");
        if let Some(ui) = window() {
            ui.global::<State>().set_busy(true);
        }
        let (server, user, pass) = (server.to_string(), user.to_string(), pass.to_string());
        background(move || Client::login(&server, &user, &pass), |r| match r {
            Ok(c) => load_library(c),
            Err(e) => {
                if let Some(ui) = window() {
                    ui.global::<State>().set_busy(false);
                }
                say(e);
            }
        });
    });
    st.on_sign_out(|| {
        stop_book();
        save_session(None);
        with_app(|a| {
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
            save_prefs(t.get_dark(), with_app(|a| a.speed));
        }
    });
    st.on_refresh_library(refresh_library);
    st.on_covers_needed(|list, first, last| covers_needed(&list, first, last));
    st.on_open_book(|id| open_book(id.to_string()));
    st.on_play_book(|id| play_book(id.to_string()));
    st.on_set_finished(|id, f| set_finished(id.to_string(), f));

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

    let timer = Timer::default();
    let (w, t) = (ui.as_weak(), tray.as_weak());
    timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        if let (Some(ui), Some(tray)) = (w.upgrade(), t.upgrade()) {
            tick(&ui.global::<State>(), &tray);
        }
    });

    ui.show()?;
    covers::set_scale(ui.window().scale_factor()); // known once the window is on a screen, and needed before the first cover
    match load_session() {
        Some(c) => {
            ui.set_server(c.base.clone().into());
            load_library(c);
        }
        None => st.set_route("settings".into()),
    }
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
