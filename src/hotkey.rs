//! The global shortcut. Carbon hot keys need no permission and report both
//! the press and the release, which is what push-to-talk needs. Escape is
//! registered only while a dictation is running, and Enter only during
//! hands-free listening, so neither is taken from other apps otherwise.

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::config::Shortcut;

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
    escape: HotKey,
    escape_on: bool,
    enter: HotKey,
    enter_on: bool,
    numpad_enter: HotKey,
    numpad_enter_on: bool,
}

fn hotkey(shortcut: Shortcut) -> HotKey {
    let modifiers = match shortcut {
        Shortcut::CtrlOptSpace => Modifiers::CONTROL | Modifiers::ALT,
        Shortcut::OptSpace => Modifiers::ALT,
        Shortcut::CmdShiftSpace => Modifiers::META | Modifiers::SHIFT,
    };
    HotKey::new(Some(modifiers), Code::Space)
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
            escape: HotKey::new(None, Code::Escape),
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

    /// What a raw event means, if it is one of ours.
    pub fn read(&self, (id, pressed): (u32, bool)) -> Option<Press> {
        if id == self.talk.id() {
            Some(if pressed { Press::Down } else { Press::Up })
        } else if (id == self.enter.id() || id == self.numpad_enter.id()) && pressed {
            Some(Press::Finish)
        } else if id == self.escape.id() && pressed {
            Some(Press::Cancel)
        } else {
            None
        }
    }

    pub fn set_shortcut(&mut self, shortcut: Shortcut) -> Result<(), String> {
        let next = hotkey(shortcut);
        if next == self.talk && self.talk_on {
            return Ok(());
        }
        self.manager.register(next).map_err(|e| e.to_string())?;
        if self.talk_on {
            self.manager.unregister(self.talk).ok();
        }
        self.talk = next;
        self.talk_on = true;
        Ok(())
    }

    /// Take or give back Escape.
    pub fn listen_for_escape(&mut self, on: bool) {
        if on == self.escape_on {
            return;
        }
        let done = if on {
            self.manager.register(self.escape)
        } else {
            self.manager.unregister(self.escape)
        };
        if done.is_ok() {
            self.escape_on = on;
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
