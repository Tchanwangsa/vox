//! Typing into the app in front: the text goes on the clipboard and Vox
//! presses ⌘V, which macOS allows only once Vox has Accessibility access.

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

const KEY_V: u16 = 9;

/// Whether macOS lets Vox press keys in other apps.
pub fn allowed() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// System Settings, on the Accessibility list.
pub fn open_accessibility_settings() {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn()
        .ok();
}

/// System Settings, on the Microphone list.
pub fn open_microphone_settings() {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        .spawn()
        .ok();
}

/// ⌘V in the app in front.
pub fn press_paste() -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| "macOS would not let Vox press keys")?;
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), KEY_V, down)
            .map_err(|_| "macOS would not let Vox press keys")?;
        event.set_flags(CGEventFlags::CGEventFlagCommand);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}
