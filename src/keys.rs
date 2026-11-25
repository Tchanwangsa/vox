//! The Groq API key: `GROQ_API_KEY` when set, else the login keychain. Read
//! once and kept in memory, so the keychain is asked at most once a launch.
//!
//! Isolated runs (`VOX_DATA_DIR`, `VOX_DEMO`, or `VOX_NO_KEYCHAIN`) never touch
//! the login keychain, and keychain reads disable interactive prompts so an
//! unsigned dev/visual-pass binary can never spam macOS "Always Allow" dialogs.

use std::ffi::OsString;
use std::sync::Mutex;

#[cfg(target_os = "macos")]
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

const SERVICE: &str = "com.tchan.vox.groq";
const ACCOUNT: &str = "groq";

/// `errSecItemNotFound`: the normal state when no Groq key has been saved yet.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

static CACHE: Mutex<Option<Option<String>>> = Mutex::new(None);

fn is_isolated(var: impl Fn(&str) -> Option<OsString>) -> bool {
    ["VOX_DATA_DIR", "VOX_DEMO", "VOX_NO_KEYCHAIN"]
        .into_iter()
        .any(|name| var(name).is_some_and(|v| !v.is_empty()))
}

/// Development, demo, and throwaway-data runs stay isolated from the user's
/// real login keychain. Set `GROQ_API_KEY` to mock a saved key in those runs.
fn keychain_isolated() -> bool {
    is_isolated(|name| std::env::var_os(name))
}

fn read_keychain() -> Option<String> {
    if keychain_isolated() {
        return None;
    }
    // Never pop a modal Keychain ACL dialog on startup or from an unsigned
    // binary. Signed builds (`Vox.app` and `cargo run`, both signed with the
    // same identity and `com.tchan.vox` identifier) satisfy the item's ACL and
    // succeed silently.
    #[cfg(target_os = "macos")]
    let _no_ui = SecKeychain::disable_user_interaction().ok();
    match get_generic_password(SERVICE, ACCOUNT) {
        Ok(bytes) => String::from_utf8(bytes).ok(),
        Err(err) => {
            if err.code() != ERR_SEC_ITEM_NOT_FOUND {
                eprintln!(
                    "[vox] Groq Keychain skipped ({err}); sign with scripts/sign.sh, run via `cargo run`, or set GROQ_API_KEY"
                );
            }
            None
        }
    }
}

pub fn groq() -> Option<String> {
    let mut cache = CACHE.lock().unwrap();
    cache
        .get_or_insert_with(|| {
            std::env::var("GROQ_API_KEY")
                .ok()
                .filter(|key| !key.trim().is_empty())
                .or_else(read_keychain)
        })
        .clone()
}

pub fn set_groq(key: &str) -> Result<(), String> {
    if !keychain_isolated() {
        set_generic_password(SERVICE, ACCOUNT, key.as_bytes()).map_err(|e| {
            eprintln!("[vox] Groq Keychain: {e}");
            "Couldn't save the Groq key to Keychain".to_string()
        })?;
    }
    *CACHE.lock().unwrap() = Some(Some(key.to_string()));
    Ok(())
}

pub fn remove_groq() {
    if !keychain_isolated() {
        delete_generic_password(SERVICE, ACCOUNT).ok();
    }
    *CACHE.lock().unwrap() = Some(None);
}

/// `gsk_…a1b2`: enough to tell two keys apart, never the key.
pub fn masked(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("gsk_…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_keeps_only_the_last_four_characters() {
        assert_eq!(masked("gsk_secret_value_a1b2"), "gsk_…a1b2");
    }

    #[test]
    fn isolation_triggers_on_dev_or_demo_env_vars_only_when_non_empty() {
        assert!(!is_isolated(|_| None));
        assert!(!is_isolated(|_| Some(OsString::new())));
        assert!(is_isolated(|name| (name == "VOX_DATA_DIR").then(|| OsString::from("/tmp/vox"))));
        assert!(is_isolated(|name| (name == "VOX_DEMO").then(|| OsString::from("listening"))));
        assert!(is_isolated(|name| (name == "VOX_NO_KEYCHAIN").then(|| OsString::from("1"))));
    }
}
