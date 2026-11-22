// Updates: reads latest.json from the repository's `updates` branch, and downloads and installs a newer signed release.
// The signature check uses the same key as the Tauri app, so a release is only installed if it was signed by our release workflow.
use cargo_packager_updater::{semver::Version, url::Url, Config, Update, UpdaterBuilder};

const ENDPOINT: &str = "https://raw.githubusercontent.com/pranoyTen/lectern/updates/latest.json";
const PUBKEY: &str = include_str!("../updater.pub");

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A newer release, or None when this is the latest.
pub fn check() -> Result<Option<Update>, String> {
    let version = Version::parse(current()).map_err(|e| e.to_string())?;
    // read the version first: an update file from an older release (a 1.x one from the Tauri app has no `format` fields, which the
    // updater requires) just means there is nothing newer, not an error
    let res = reqwest::blocking::Client::new().get(ENDPOINT).send().map_err(|_| "Can't reach GitHub. Check your connection".to_string())?;
    if !res.status().is_success() {
        return Err("Couldn't read the update. Check that a release has been published".into());
    }
    let manifest: serde_json::Value = res.json().map_err(|e| format!("Couldn't read the update: {e}"))?;
    if !is_newer(manifest["version"].as_str().unwrap_or_default(), &version) {
        return Ok(None);
    }
    let config = Config { endpoints: vec![Url::parse(ENDPOINT).unwrap()], pubkey: PUBKEY.trim().into(), windows: None };
    let updater = UpdaterBuilder::new(version, config).build().map_err(explain)?;
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
        "Couldn't read the update. Check that a release has been published".into()
    } else if l.contains("signature") {
        "The update's signature didn't match, so it wasn't installed".into()
    } else if l.contains("dns") || l.contains("connect") || l.contains("timed out") || l.contains("network") {
        "Can't reach GitHub. Check your connection".into()
    } else {
        format!("Couldn't update: {m}")
    }
}

/// Whether the update file's version ("2.1.0", or "v2.1.0") is newer than this one.
fn is_newer(remote: &str, current: &Version) -> bool {
    Version::parse(remote.trim_start_matches('v')).is_ok_and(|v| v > *current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_newer_release_is_an_update() {
        let now = Version::parse("2.0.0").unwrap();
        assert!(!is_newer("1.2.1", &now)); // the Tauri app's last release
        assert!(!is_newer("2.0.0", &now));
        assert!(is_newer("2.0.1", &now));
        assert!(is_newer("v2.1.0", &now));
        assert!(!is_newer("", &now));
    }
}
