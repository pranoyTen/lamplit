// Updates: reads latest.json from the repository's `updates` branch, and downloads and installs a newer signed release.
// The repository is private for now, so both the check and the download send the user's GitHub token (Settings > Updates); the
// signature check uses the same key as the Tauri app, so a release is only installed if it was signed by our release workflow.
use cargo_packager_updater::{semver::Version, url::Url, Config, Update, UpdaterBuilder};

const ENDPOINT: &str = "https://raw.githubusercontent.com/pranoyTen/lectern/updates/latest.json";
const PUBKEY: &str = include_str!("../updater.pub");

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A newer release, or None when this is the latest.
pub fn check(token: &str) -> Result<Option<Update>, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Add a GitHub token first: the repository is private, so updates need one".into());
    }
    let config = Config { endpoints: vec![Url::parse(ENDPOINT).unwrap()], pubkey: PUBKEY.trim().into(), windows: None };
    let version = Version::parse(current()).map_err(|e| e.to_string())?;
    let updater = UpdaterBuilder::new(version, config)
        .header("Authorization", format!("Bearer {token}"))
        .and_then(|b| b.build())
        .map_err(explain)?;
    updater.check().map_err(explain)
}

/// Downloads the release, checks its signature and starts its installer. On Windows the installer takes over and this process exits.
pub fn install(u: &Update) -> Result<(), String> {
    u.download_and_install().map_err(explain)
}

/// Turns updater errors into something a listener can act on, like the Tauri app's explainUpdateError.
pub fn explain(e: cargo_packager_updater::Error) -> String {
    let m = e.to_string();
    let l = m.to_lowercase();
    if l.contains("401") || l.contains("403") || l.contains("404") {
        "Couldn't read the update. Check that the token is valid and a release has been published".into()
    } else if l.contains("signature") {
        "The update's signature didn't match, so it wasn't installed".into()
    } else if l.contains("dns") || l.contains("connect") || l.contains("timed out") || l.contains("network") {
        "Can't reach GitHub. Check your connection".into()
    } else {
        format!("Couldn't update: {m}")
    }
}
