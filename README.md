# Lectern

> [!NOTE]
> **How Lectern is made:** I built Lectern for my own listening and share it in case it's useful to you. Because it started as a personal tool, most of the code is written with Claude Code rather than by hand. I choose every feature, read every diff before it's merged, and use each change in the app myself before it goes into a release.

Lectern is a Windows and Mac app for listening to your own [Audiobookshelf](https://www.audiobookshelf.org/) library. It streams or downloads your books and keeps your place in sync with every other device you listen on.

It is written in Rust with [Slint](https://slint.dev/) and has no web view inside, so it uses about 20 to 45 MB of memory while playing, where the earlier web-based version used about 400 MB.

<!-- Screenshot of Home goes here, e.g. ![Lectern's Home screen](docs/home.png) -->

Lectern is not affiliated with the Audiobookshelf project. It is an independent client and needs an Audiobookshelf server to connect to.

## Features

**Listening**
- Streams from your server in small pieces, so a 30-hour book starts without downloading it first
- Playback speed from 0.75× to 2×, remembered per book, with pitch kept natural
- Skip back 10, 15 or 30 seconds and forward 15, 30 or 60 seconds
- Rewinds 10 seconds when you resume after a break of over a minute
- Chapter list, previous and next chapter, and an option to stop at the end of each chapter
- Bookmarks, saved to your server so they show up in other apps
- Progress synced with your server, so you pick up where you left off on any device (can be turned off)
- Sleep timer: 15, 30, 45 or 60 minutes, or the end of the chapter
- A default sleep timer that starts whenever you press play
- The sleep timer fades out over its last 30 seconds, and +10 min extends it
- Five-band equalizer with presets (Flat, Voice, Less bass, More bass, Treble)

**Library**
- Home with the book you're listening to, Continue listening and Recently added
- Library with search, filters (in progress, not started, finished, downloaded) and sorting (title, author, recently added, longest)
- Series, and a page for each book with its details and description
- Show or hide each of your server's libraries
- Covers made from the original image (often 2400 px) rather than the server's 400 px preview, and kept on disk so they aren't downloaded again

**Offline**
- Download books to listen without a connection, with resumable downloads
- Your place is kept on this computer while you're offline
- A storage limit (5, 10, 20 or 50 GB, or none), with finished books removed automatically if you want
- Waits for an unmetered connection before downloading, unless you allow it
- Choose where downloads are stored

**Stats**
- Listening time for the week, month or year, with a daily or monthly chart
- Your listening streak, most listened authors and recently finished books

**Desktop**
- Media keys and the system's now playing controls (the Windows media flyout and macOS Now Playing)
- System tray icon with a mini player; closing the window keeps Lectern playing in the tray
- Keyboard shortcuts: Space to play or pause, Left and Right to skip, Up and Down for volume
- Light and dark themes
- Updates itself: checks at launch and installs with one click

## Install

Download the installer for your system from the [latest release](../../releases/latest):

| System | File |
| --- | --- |
| Windows 10 and 11 | `lectern_<version>_x64-setup.exe` |
| macOS on Apple silicon | `Lectern_aarch64.dmg` |
| macOS on Intel | `Lectern_x64.dmg` |

The installers are not code-signed, so Windows SmartScreen and macOS Gatekeeper warn the first time you open them. On Windows choose **More info › Run anyway**. On a Mac, try to open Lectern once, then go to **System Settings › Privacy & Security** and choose **Open Anyway**.

Then open Lectern, enter your server address, username and password, and your libraries appear.

### Updates

Lectern checks for a new release at every launch and offers to install it from the sidebar. You can also check in **Settings › Updates**.

## Building from source

You need [Rust](https://rustup.rs/) (stable). Windows and macOS are the supported systems; Linux may build (it needs the ALSA development package, `libasound2-dev` on Debian and Ubuntu) but isn't tested.

```sh
git clone https://github.com/pranoyTen/lectern.git
cd lectern/native
cargo run --release
```

Run the tests with `cargo test`. The installers are built by [cargo-packager](https://github.com/crabnebula-dev/cargo-packager):

```sh
cargo install cargo-packager --locked
cargo packager --release
```

## Releasing

Releases are built by GitHub Actions (`.github/workflows/release-native.yml`). Set the version in `native/Cargo.toml`, then push a matching tag:

```sh
git tag v2.0.1
git push origin v2.0.1
```

The workflow builds Windows and both Mac versions, signs the update files, attaches everything to a draft release and publishes the update information that installed copies check. Publish the draft to release it.

## Project layout

| Path | What it holds |
| --- | --- |
| `native/src` | The app: playback engine, Audiobookshelf API client, downloads, covers, updater |
| `native/ui` | The interface, in Slint |
| `native/icons` | App and tray icons |
| `scripts`, `tests` | Builds the update information for each release, and its tests |

## License

Lectern is free software under the [GNU General Public License v3.0](LICENSE). You can use, change and share it, and if you share a changed version, you share its source under the same license.

The interface is built with [Slint](https://slint.dev/), used under its GPL-3.0 license.
