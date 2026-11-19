// Lectern, native: a Slint window over the playback core. Network calls run on worker threads and hand their results back to the UI
// thread with invoke_from_event_loop; everything the UI owns lives in APP on that thread.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod api;
mod http_source;
mod media;
mod player;
mod stretch;

use api::{Book, Client};
use player::Engine;
use souvlaki::{MediaControlEvent, SeekDirection};
use slint::{ComponentHandle, ModelRc, SharedString, StandardListViewItem, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

slint::include_modules!();

struct Playing {
    engine: Engine,
    session: String,
    title: String,
    duration: f64,
    listened: f64, // seconds heard since the last report to the server
    last_tick: Instant,
}

struct App {
    _sink: rodio::MixerDeviceSink,
    out: rodio::Player,
    client: Option<Client>,
    books: Vec<Book>,
    playing: Option<Playing>,
    speed: f64,
    media: Option<souvlaki::MediaControls>,
    shown: (Option<bool>, Instant), // the play state last published to the OS overlay and the tray, and when
}

// one set of actions for the window's buttons, the tray menu and the media keys
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
    if with_app(|a| a.out.is_paused()) != paused {
        toggle();
    }
}

fn skip(d: f64) {
    with_app(|a| if let Some(p) = &a.playing { p.engine.seek((p.engine.status.pos() + d).min(p.duration)) });
}

fn seek(t: f64) {
    with_app(|a| if let Some(p) = &a.playing { p.engine.seek(t) });
}

// Next and Previous have no chapters to move between yet, so they skip forward 30 and back 15 like the buttons
fn media_event(e: MediaControlEvent) {
    match e {
        MediaControlEvent::Play => set_paused(false),
        MediaControlEvent::Pause | MediaControlEvent::Stop => set_paused(true),
        MediaControlEvent::Toggle => toggle(),
        MediaControlEvent::Next | MediaControlEvent::Seek(SeekDirection::Forward) => skip(30.0),
        MediaControlEvent::Previous | MediaControlEvent::Seek(SeekDirection::Backward) => skip(-15.0),
        MediaControlEvent::SeekBy(SeekDirection::Forward, d) => skip(d.as_secs_f64()),
        MediaControlEvent::SeekBy(SeekDirection::Backward, d) => skip(-d.as_secs_f64()),
        MediaControlEvent::SetPosition(p) => seek(p.0.as_secs_f64()),
        _ => {}
    }
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(a.borrow_mut().as_mut().expect("the app is set up before the window runs")))
}

// where the sign-in is remembered between runs
// shortcut: the token sits in a plain file in the user's config folder, as it did in the Tauri app's local storage; move it to the OS
// keychain (keyring crate) before release
fn config_path() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("lectern-native").join("session.json"))
}

fn save_session(c: Option<&Client>) {
    let Some(p) = config_path() else { return };
    let _ = match c {
        Some(c) => std::fs::create_dir_all(p.parent().unwrap()).and_then(|_| std::fs::write(&p, serde_json::json!({ "server": c.base, "token": c.token }).to_string())),
        None => std::fs::remove_file(&p),
    };
}

fn load_session() -> Option<Client> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config_path()?).ok()?).ok()?;
    Client::new(v["server"].as_str()?, v["token"].as_str()?).ok()
}

/// The tray icon as RGBA pixels, decoded from the PNG built into the exe.
fn tray_icon() -> Option<slint::Image> {
    let mut d = png::Decoder::new(std::io::Cursor::new(include_bytes!("../ui/icons/tray.png")));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let px = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&buf[..info.buffer_size()], info.width, info.height);
    Some(slint::Image::from_rgba8(px))
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

fn show_books(ui: &AppWindow, client: Client) {
    ui.set_busy(true);
    ui.set_message("Loading books…".into());
    let weak = ui.as_weak();
    background(
        move || client.books().map(|b| (client, b)),
        move |r| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_busy(false);
            match r {
                Ok((client, books)) => {
                    let items: Vec<StandardListViewItem> = books.iter().map(|b| StandardListViewItem::from(format!("{} — {}", b.title, b.author).as_str())).collect();
                    ui.set_books(ModelRc::new(VecModel::from(items)));
                    ui.set_message(if books.is_empty() { "No books found".into() } else { SharedString::new() });
                    ui.set_signed_in(true);
                    save_session(Some(&client));
                    with_app(|a| (a.client, a.books) = (Some(client), books));
                }
                Err(e) => ui.set_message(e.into()),
            }
        },
    );
}

fn play_book(ui: &AppWindow, i: usize) {
    let Some((client, book)) = with_app(|a| Some((a.client.clone()?, a.books.get(i)?.clone()))) else { return };
    stop_book();
    ui.set_message(format!("Opening {}…", book.title).into());
    let weak = ui.as_weak();
    background(
        { let id = book.id.clone(); move || client.play(&id).map(|s| (client, s)) },
        move |r| {
            let Some(ui) = weak.upgrade() else { return };
            let (client, s) = match r {
                Ok(v) => v,
                Err(e) => return ui.set_message(e.into()),
            };
            ui.set_message(SharedString::new());
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
            ui.set_now_title(book.title.clone().into());
            ui.set_duration(s.duration as f32);
            ui.set_has_book(true);
            let cover = client.cover_url(&book.id);
            with_app(|a| {
                if let Some(m) = a.media.as_mut() {
                    media::show_book(m, &book.title, &book.author, &cover, s.duration);
                }
                a.shown.0 = None;
                a.playing = Some(Playing { engine, session: s.id, title: book.title, duration: s.duration, listened: 0.0, last_tick: Instant::now() });
            });
        },
    );
}

// four times a second: move the position bar, show errors and the end of the book, keep the OS overlay and the tray in step,
// and report progress every 15 seconds heard
fn tick(ui: &AppWindow, tray: &Tray) {
    let due = with_app(|a| {
        let paused = a.out.is_paused();
        let Some(p) = a.playing.as_mut() else { return false };
        let playing = !paused && p.engine.status.state.load(Relaxed) == player::PLAYING;
        // the overlay moves its own timeline while playing, so it only needs the state on a change and a correction now and then
        if a.shown.0 != Some(playing) || a.shown.1.elapsed() > Duration::from_secs(5) {
            if let Some(m) = a.media.as_mut() {
                media::show_state(m, Some(playing), p.engine.status.pos());
            }
            tray.set_playing(playing);
            a.shown = (Some(playing), Instant::now());
        }
        let now = Instant::now();
        let state = p.engine.status.state.load(Relaxed);
        if !paused && state == player::PLAYING {
            p.listened += (now - p.last_tick).as_secs_f64();
        }
        p.last_tick = now;
        let pos = p.engine.status.pos();
        if !ui.get_seeking() {
            ui.set_position(pos as f32);
        }
        ui.set_pos_text(clock(pos).into());
        ui.set_left_text(format!("-{}", clock(p.duration - pos)).into());
        ui.set_playing(!paused && state == player::PLAYING);
        match state {
            player::FAILED => ui.set_message(p.engine.status.error.lock().unwrap().clone().into()),
            player::ENDED => ui.set_message(format!("Finished {}", p.title).into()),
            _ => {}
        }
        p.listened >= 15.0
    });
    if due {
        report(false);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sink = rodio::DeviceSinkBuilder::open_default_sink()?;
    sink.log_on_drop(false);
    let out = rodio::Player::connect_new(sink.mixer());
    APP.with(|a| *a.borrow_mut() = Some(App { _sink: sink, out, client: None, books: vec![], playing: None, speed: 1.0, media: None, shown: (None, Instant::now()) }));

    let ui = AppWindow::new()?;

    let w = ui.as_weak();
    ui.on_sign_in(move || {
        let ui = w.unwrap();
        let (server, user, pass) = (ui.get_server().to_string(), ui.get_username().to_string(), ui.get_password().to_string());
        ui.set_busy(true);
        ui.set_message("Signing in…".into());
        let w = ui.as_weak();
        background(move || Client::login(&server, &user, &pass), move |r| {
            let Some(ui) = w.upgrade() else { return };
            ui.set_password(SharedString::new());
            match r {
                Ok(c) => show_books(&ui, c),
                Err(e) => {
                    ui.set_busy(false);
                    ui.set_message(e.into());
                }
            }
        });
    });
    let w = ui.as_weak();
    ui.on_sign_out(move || {
        stop_book();
        save_session(None);
        with_app(|a| (a.client, a.books) = (None, vec![]));
        let ui = w.unwrap();
        ui.set_books(ModelRc::new(VecModel::<StandardListViewItem>::default()));
        ui.set_has_book(false);
        ui.set_signed_in(false);
    });
    let w = ui.as_weak();
    ui.on_pick(move |i| play_book(&w.unwrap(), i as usize));
    ui.on_toggle(toggle);
    ui.on_skip(|d| skip(d as f64));
    ui.on_seek(|t| seek(t as f64));
    ui.on_speed(|v| {
        let s: f64 = v.trim_end_matches('×').parse().unwrap_or(1.0);
        with_app(|a| {
            a.speed = s;
            if let Some(p) = &a.playing {
                p.engine.set_speed(s);
            }
        });
    });

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
            tick(&ui, &tray);
        }
    });

    if let Some(c) = load_session() {
        ui.set_server(c.base.clone().into());
        show_books(&ui, c);
    }
    ui.show()?;
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
