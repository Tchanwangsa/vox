//! The global shortcut. Carbon hot keys need no permission and report both
//! the press and the release, which is what push-to-talk needs. Escape is
//! registered only while a dictation is running, and Enter only during
//! hands-free listening, so neither is taken from other apps otherwise.

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::config::{Shortcut, ShortcutKey, ShortcutModifiers};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Press {
    Down,
    Up,
    Finish,
    Cancel,
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    talk: HotKey,
    /// False when another app held the shortcut and it could not be taken.
    talk_on: bool,
    /// True while the user is recording a new custom shortcut in Settings.
    paused: bool,
    /// Escape chords currently registered (bare Escape plus subsets of `talk`'s modifiers).
    active_escapes: Vec<HotKey>,
    escape_on: bool,
    enter: HotKey,
    enter_on: bool,
    numpad_enter: HotKey,
    numpad_enter_on: bool,
}

fn modifiers(mods: ShortcutModifiers) -> Option<Modifiers> {
    let mut out = Modifiers::empty();
    if mods.control {
        out |= Modifiers::CONTROL;
    }
    if mods.alt {
        out |= Modifiers::ALT;
    }
    if mods.shift {
        out |= Modifiers::SHIFT;
    }
    if mods.command {
        out |= Modifiers::META;
    }
    (!out.is_empty()).then_some(out)
}

fn code(key: ShortcutKey) -> Code {
    match key {
        ShortcutKey::A => Code::KeyA,
        ShortcutKey::B => Code::KeyB,
        ShortcutKey::C => Code::KeyC,
        ShortcutKey::D => Code::KeyD,
        ShortcutKey::E => Code::KeyE,
        ShortcutKey::F => Code::KeyF,
        ShortcutKey::G => Code::KeyG,
        ShortcutKey::H => Code::KeyH,
        ShortcutKey::I => Code::KeyI,
        ShortcutKey::J => Code::KeyJ,
        ShortcutKey::K => Code::KeyK,
        ShortcutKey::L => Code::KeyL,
        ShortcutKey::M => Code::KeyM,
        ShortcutKey::N => Code::KeyN,
        ShortcutKey::O => Code::KeyO,
        ShortcutKey::P => Code::KeyP,
        ShortcutKey::Q => Code::KeyQ,
        ShortcutKey::R => Code::KeyR,
        ShortcutKey::S => Code::KeyS,
        ShortcutKey::T => Code::KeyT,
        ShortcutKey::U => Code::KeyU,
        ShortcutKey::V => Code::KeyV,
        ShortcutKey::W => Code::KeyW,
        ShortcutKey::X => Code::KeyX,
        ShortcutKey::Y => Code::KeyY,
        ShortcutKey::Z => Code::KeyZ,
        ShortcutKey::Digit0 => Code::Digit0,
        ShortcutKey::Digit1 => Code::Digit1,
        ShortcutKey::Digit2 => Code::Digit2,
        ShortcutKey::Digit3 => Code::Digit3,
        ShortcutKey::Digit4 => Code::Digit4,
        ShortcutKey::Digit5 => Code::Digit5,
        ShortcutKey::Digit6 => Code::Digit6,
        ShortcutKey::Digit7 => Code::Digit7,
        ShortcutKey::Digit8 => Code::Digit8,
        ShortcutKey::Digit9 => Code::Digit9,
        ShortcutKey::Space => Code::Space,
        ShortcutKey::Tab => Code::Tab,
        ShortcutKey::Enter => Code::Enter,
        ShortcutKey::Backspace => Code::Backspace,
        ShortcutKey::Delete => Code::Delete,
        ShortcutKey::Escape => Code::Escape,
        ShortcutKey::ArrowUp => Code::ArrowUp,
        ShortcutKey::ArrowDown => Code::ArrowDown,
        ShortcutKey::ArrowLeft => Code::ArrowLeft,
        ShortcutKey::ArrowRight => Code::ArrowRight,
        ShortcutKey::Home => Code::Home,
        ShortcutKey::End => Code::End,
        ShortcutKey::PageUp => Code::PageUp,
        ShortcutKey::PageDown => Code::PageDown,
        ShortcutKey::Backquote => Code::Backquote,
        ShortcutKey::Minus => Code::Minus,
        ShortcutKey::Equal => Code::Equal,
        ShortcutKey::BracketLeft => Code::BracketLeft,
        ShortcutKey::BracketRight => Code::BracketRight,
        ShortcutKey::Backslash => Code::Backslash,
        ShortcutKey::Semicolon => Code::Semicolon,
        ShortcutKey::Quote => Code::Quote,
        ShortcutKey::Comma => Code::Comma,
        ShortcutKey::Period => Code::Period,
        ShortcutKey::Slash => Code::Slash,
        ShortcutKey::F1 => Code::F1,
        ShortcutKey::F2 => Code::F2,
        ShortcutKey::F3 => Code::F3,
        ShortcutKey::F4 => Code::F4,
        ShortcutKey::F5 => Code::F5,
        ShortcutKey::F6 => Code::F6,
        ShortcutKey::F7 => Code::F7,
        ShortcutKey::F8 => Code::F8,
        ShortcutKey::F9 => Code::F9,
        ShortcutKey::F10 => Code::F10,
        ShortcutKey::F11 => Code::F11,
        ShortcutKey::F12 => Code::F12,
        ShortcutKey::F13 => Code::F13,
        ShortcutKey::F14 => Code::F14,
        ShortcutKey::F15 => Code::F15,
        ShortcutKey::F16 => Code::F16,
        ShortcutKey::F17 => Code::F17,
        ShortcutKey::F18 => Code::F18,
        ShortcutKey::F19 => Code::F19,
        ShortcutKey::F20 => Code::F20,
    }
}

pub fn hotkey(shortcut: Shortcut) -> HotKey {
    HotKey::new(modifiers(shortcut.modifiers), code(shortcut.key))
}

/// Escape chords to register while a dictation is running: bare Escape plus
/// Escape combined with any subset of the talk shortcut's modifiers, so
/// pressing Escape cancels in hold-to-talk mode even while the talk modifiers
/// are still held. Excludes `talk` itself if the user chose an Escape shortcut.
fn escape_hotkeys(talk: HotKey) -> Vec<HotKey> {
    let bits: Vec<Modifiers> = [
        Modifiers::CONTROL,
        Modifiers::ALT,
        Modifiers::SHIFT,
        Modifiers::META,
    ]
    .into_iter()
    .filter(|m| talk.mods.contains(*m))
    .collect();

    let mut out = Vec::with_capacity(1 << bits.len());
    for mask in 0..(1usize << bits.len()) {
        let mut mods = Modifiers::empty();
        for (i, &bit) in bits.iter().enumerate() {
            if (mask & (1 << i)) != 0 {
                mods |= bit;
            }
        }
        let hk = HotKey::new((!mods.is_empty()).then_some(mods), Code::Escape);
        if hk != talk {
            out.push(hk);
        }
    }
    out
}

impl Hotkeys {
    /// Must be made on the main thread. The receiver yields raw events for as
    /// long as the app runs; [`Hotkeys::taken`] says whether the shortcut is ours.
    pub fn new(shortcut: Shortcut) -> Result<(Self, UnboundedReceiver<(u32, bool)>), String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
        let talk = hotkey(shortcut);
        let talk_on = manager.register(talk).is_ok();
        let (tx, rx) = unbounded();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            tx.unbounded_send((event.id(), event.state() == HotKeyState::Pressed)).ok();
        }));
        let hotkeys = Self {
            manager,
            talk,
            talk_on,
            paused: false,
            active_escapes: Vec::new(),
            escape_on: false,
            enter: HotKey::new(None, Code::Enter),
            enter_on: false,
            numpad_enter: HotKey::new(None, Code::NumpadEnter),
            numpad_enter_on: false,
        };
        Ok((hotkeys, rx))
    }

    pub fn taken(&self) -> bool {
        self.talk_on
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Suspend the global talk shortcut while recording a new one, or restore
    /// it when recording ends.
    pub fn set_paused(&mut self, paused: bool) {
        if paused == self.paused {
            return;
        }
        self.paused = paused;
        if paused {
            if self.talk_on {
                self.manager.unregister(self.talk).ok();
                self.talk_on = false;
            }
        } else if !self.talk_on {
            self.talk_on = self.manager.register(self.talk).is_ok();
        }
    }

    /// What a raw event means, if it is one of ours.
    pub fn read(&self, (id, pressed): (u32, bool)) -> Option<Press> {
        if self.paused {
            return None;
        }
        if id == self.talk.id() {
            Some(if pressed { Press::Down } else { Press::Up })
        } else if (id == self.enter.id() || id == self.numpad_enter.id()) && pressed {
            Some(Press::Finish)
        } else if pressed && self.active_escapes.iter().any(|hk| hk.id() == id) {
            Some(Press::Cancel)
        } else {
            None
        }
    }

    pub fn set_shortcut(&mut self, shortcut: Shortcut) -> Result<(), String> {
        let next = hotkey(shortcut);
        if !self.paused && next == self.talk && self.talk_on {
            return Ok(());
        }
        let was_escaping = self.escape_on;
        if was_escaping {
            self.listen_for_escape(false);
        }
        if let Err(e) = self.manager.register(next) {
            if was_escaping {
                self.listen_for_escape(true);
            }
            return Err(e.to_string());
        }
        if self.talk_on {
            self.manager.unregister(self.talk).ok();
        }
        self.talk = next;
        self.talk_on = true;
        self.paused = false;
        if was_escaping {
            self.listen_for_escape(true);
        }
        Ok(())
    }

    /// Take or give back Escape (bare and with any subset of `talk`'s modifiers).
    pub fn listen_for_escape(&mut self, on: bool) {
        if on == self.escape_on {
            return;
        }
        self.escape_on = on;
        if on {
            for hk in escape_hotkeys(self.talk) {
                if self.manager.register(hk).is_ok() {
                    self.active_escapes.push(hk);
                }
            }
        } else {
            for hk in self.active_escapes.drain(..) {
                self.manager.unregister(hk).ok();
            }
        }
    }

    /// Take or give back Enter / Numpad Enter (hands-free finish).
    pub fn listen_for_enter(&mut self, on: bool) {
        if on != self.enter_on {
            let done = if on {
                self.manager.register(self.enter)
            } else {
                self.manager.unregister(self.enter)
            };
            if done.is_ok() {
                self.enter_on = on;
            }
        }
        if on != self.numpad_enter_on {
            let done = if on {
                self.manager.register(self.numpad_enter)
            } else {
                self.manager.unregister(self.numpad_enter)
            };
            if done.is_ok() {
                self.numpad_enter_on = on;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shortcut_key_maps_to_a_distinct_hotkey_and_round_trips_tokens() {
        let mut ids = std::collections::HashSet::new();
        for key in ShortcutKey::ALL {
            let parsed = ShortcutKey::from_token(key.token()).expect("token round-trip");
            assert_eq!(parsed, key);
            let hk = hotkey(Shortcut::new(ShortcutModifiers::ALT, key));
            assert!(ids.insert(hk.id()), "duplicate hotkey id for {key:?}");
        }
    }

    #[test]
    fn escape_chords_cover_talk_modifier_subsets_without_colliding_with_talk() {
        let ctrl_opt_space = hotkey(Shortcut::CTRL_OPT_SPACE);
        let chords = escape_hotkeys(ctrl_opt_space);
        assert_eq!(
            chords,
            vec![
                HotKey::new(None, Code::Escape),
                HotKey::new(Some(Modifiers::CONTROL), Code::Escape),
                HotKey::new(Some(Modifiers::ALT), Code::Escape),
                HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Escape),
            ]
        );

        let f5 = hotkey(Shortcut::new(ShortcutModifiers::NONE, ShortcutKey::F5));
        assert_eq!(escape_hotkeys(f5), vec![HotKey::new(None, Code::Escape)]);

        let opt_esc = hotkey(Shortcut::new(ShortcutModifiers::ALT, ShortcutKey::Escape));
        assert_eq!(escape_hotkeys(opt_esc), vec![HotKey::new(None, Code::Escape)]);
    }
}
