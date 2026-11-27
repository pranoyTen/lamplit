// One Lamplit at a time. The running copy holds a lock on a file, which the system releases however it ends; a second copy leaves a file
// asking the first to show its window, and quits.
use std::fs::{self, File, TryLockError};
use std::path::Path;
use std::sync::Mutex;

static HELD: Mutex<Option<File>> = Mutex::new(None);

/// Whether this copy may run. If the lock can't be used at all it may: two copies are better than none.
pub fn claim() -> bool {
    let Some(d) = crate::config_dir() else { return true };
    match lock(&d) {
        Ok(f) => *HELD.lock().unwrap() = f,
        Err(()) => return false,
    }
    true
}

/// Lets another copy run, before this one quits.
pub fn release() {
    HELD.lock().unwrap().take();
}

/// Whether another copy was started since the last call.
pub fn asked() -> bool {
    crate::config_dir().is_some_and(|d| take_request(&d))
}

fn lock(dir: &Path) -> Result<Option<File>, ()> {
    let _ = fs::create_dir_all(dir);
    let Ok(f) = File::create(dir.join("running.lock")) else { return Ok(None) };
    match f.try_lock() {
        Ok(()) => {
            take_request(dir); // one nobody answered
            Ok(Some(f))
        }
        Err(TryLockError::WouldBlock) => {
            // Windows only lets the first copy come to the front if this one, just started by the user, allows it
            #[cfg(windows)]
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
                AllowSetForegroundWindow(ASFW_ANY);
            }
            let _ = fs::write(dir.join("show"), "");
            Err(())
        }
        Err(TryLockError::Error(_)) => Ok(None),
    }
}

fn take_request(dir: &Path) -> bool {
    fs::remove_file(dir.join("show")).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_copy_asks_the_first() {
        let d = std::env::temp_dir().join(format!("lamplit-instance-{}", std::process::id()));
        let first = lock(&d).unwrap().expect("locked");
        assert!(!take_request(&d));
        assert!(lock(&d).is_err());
        assert!(take_request(&d) && !take_request(&d));
        drop(first);
        assert!(lock(&d).unwrap().is_some());
        let _ = fs::remove_dir_all(&d);
    }
}
