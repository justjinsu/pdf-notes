//! Keyboard shortcuts the person can change: each action has one key
//! combination. Keys are matched by their position on the keyboard, so a
//! shortcut works the same with the Korean or the English input source on.

use std::collections::HashMap;

use eframe::egui::{self, Key, Modifiers};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Highlight,
    Underline,
    Strike,
    Note,
    Capture,
    Select,
    NextDoc,
    PrevDoc,
    OtherPane,
    ToggleSplit,
    SyncObsidian,
    OpenObsidian,
    CopyCitation,
    ToggleDark,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::Highlight,
        Action::Underline,
        Action::Strike,
        Action::Note,
        Action::Capture,
        Action::Select,
        Action::NextDoc,
        Action::PrevDoc,
        Action::OtherPane,
        Action::ToggleSplit,
        Action::SyncObsidian,
        Action::OpenObsidian,
        Action::CopyCitation,
        Action::ToggleDark,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::Highlight => "형광펜 (고른 글에 칠하기 / 도구)",
            Action::Underline => "밑줄",
            Action::Strike => "취소선",
            Action::Note => "메모",
            Action::Capture => "캡처 (이미지 선택)",
            Action::Select => "선택 도구",
            Action::NextDoc => "다음 문서로",
            Action::PrevDoc => "이전 문서로",
            Action::OtherPane => "나란히 보기에서 다른 쪽 문서로",
            Action::ToggleSplit => "나란히 보기 켜기/끄기",
            Action::SyncObsidian => "Obsidian 동기화",
            Action::OpenObsidian => "Obsidian에서 열기",
            Action::CopyCitation => "인용(APA) 복사",
            Action::ToggleDark => "어두운 화면",
        }
    }

    pub fn default_binding(self) -> Option<Binding> {
        let b = |key: Key, command: bool, shift: bool, alt: bool| Some(Binding { key: key.name().to_string(), command, shift, alt, ctrl: false });
        match self {
            Action::Highlight => b(Key::H, false, false, false),
            Action::Underline => b(Key::U, false, false, false),
            Action::Strike => b(Key::S, false, false, false),
            Action::Note => b(Key::N, false, false, false),
            Action::Capture => b(Key::C, false, false, false),
            Action::Select => b(Key::V, false, false, false),
            Action::NextDoc => b(Key::CloseBracket, true, true, false),
            Action::PrevDoc => b(Key::OpenBracket, true, true, false),
            Action::OtherPane => b(Key::Backtick, false, false, true),
            Action::ToggleSplit => b(Key::Backslash, true, false, false),
            Action::SyncObsidian => b(Key::O, true, true, false),
            Action::OpenObsidian => b(Key::O, true, true, true),
            Action::CopyCitation => b(Key::C, true, true, false),
            Action::ToggleDark => b(Key::D, true, true, false),
        }
    }
}

/// Shortcut hints are written the Mac way ("⌘⇧Z"); elsewhere they read "Ctrl+Shift+Z".
pub fn plat(s: &str) -> String {
    if cfg!(target_os = "macos") || cfg!(target_os = "ios") {
        s.to_string()
    } else {
        s.replace("⌘⇧", "Ctrl+Shift+").replace('⌘', "Ctrl+")
    }
}

/// (⌘, ⌃, ⌥, ⇧) on a Mac; elsewhere Ctrl counts as ⌘.
fn norm(m: Modifiers) -> (bool, bool, bool, bool) {
    if cfg!(target_os = "macos") {
        (m.mac_cmd || (m.command && !m.ctrl), m.ctrl, m.alt, m.shift)
    } else {
        (m.ctrl || m.command, false, m.alt, m.shift)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// egui key name ("H", "]", "F5"...).
    pub key: String,
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Binding {
    pub fn from_event(key: Key, m: Modifiers) -> Binding {
        let (command, ctrl, alt, shift) = norm(m);
        Binding { key: key.name().to_string(), command, shift, alt, ctrl }
    }

    pub fn key(&self) -> Option<Key> {
        Key::from_name(&self.key)
    }

    /// Plain keys (no ⌘/Ctrl/⌥) can't fire while typing in a text box.
    pub fn is_plain(&self) -> bool {
        !self.command && !self.ctrl && !self.alt
    }

    pub fn matches(&self, key: Key, m: Modifiers) -> bool {
        self.key().is_some_and(|k| k == key) && norm(m) == (self.command, self.ctrl, self.alt, self.shift)
    }

    pub fn label(&self) -> String {
        let mut s = String::new();
        if cfg!(target_os = "macos") {
            if self.ctrl {
                s.push('⌃');
            }
            if self.alt {
                s.push('⌥');
            }
            if self.shift {
                s.push('⇧');
            }
            if self.command {
                s.push('⌘');
            }
        } else {
            for (on, name) in [(self.ctrl || self.command, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+")] {
                if on {
                    s.push_str(name);
                }
            }
        }
        let key = self.key().map(|k| k.symbol_or_name().to_string()).unwrap_or_else(|| self.key.clone());
        s.push_str(&key);
        s
    }
}

/// The person's shortcuts; actions they never changed use the defaults.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Keymap {
    /// `None` = the person removed the shortcut.
    pub custom: HashMap<Action, Option<Binding>>,
}

impl Keymap {
    pub fn get(&self, a: Action) -> Option<Binding> {
        match self.custom.get(&a) {
            Some(b) => b.clone(),
            None => a.default_binding(),
        }
    }

    pub fn set(&mut self, a: Action, b: Option<Binding>) {
        // One combination does one thing: take it away from any other action.
        if let Some(nb) = &b {
            for other in Action::ALL {
                if other != a && self.get(other).as_ref() == Some(nb) {
                    self.custom.insert(other, None);
                }
            }
        }
        if b == a.default_binding() {
            self.custom.remove(&a);
        } else {
            self.custom.insert(a, b);
        }
    }

    pub fn reset(&mut self) {
        self.custom.clear();
    }

    pub fn label(&self, a: Action) -> String {
        self.get(a).map(|b| b.label()).unwrap_or_default()
    }

    /// Actions whose shortcut was pressed this frame. `typing` blocks plain
    /// keys so writing a comment never triggers them.
    pub fn pressed(&self, ctx: &egui::Context, typing: bool) -> Vec<Action> {
        let presses: Vec<(Key, Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers } => Some((physical_key.unwrap_or(*key), *modifiers)),
                    _ => None,
                })
                .collect()
        });
        let mut out = Vec::new();
        for (key, m) in presses {
            for a in Action::ALL {
                if let Some(b) = self.get(a) {
                    if b.matches(key, m) && !(typing && (b.is_plain() || (b.shift && !b.command && !b.ctrl && !b.alt))) {
                        out.push(a);
                    }
                }
            }
        }
        out
    }
}

/// The first key pressed this frame (for recording a new shortcut), ignoring
/// bare modifier presses. Escape cancels; Backspace/Delete clears.
pub enum Recorded {
    Nothing,
    Cancel,
    Clear,
    Set(Binding),
}

pub fn record(ctx: &egui::Context) -> Recorded {
    ctx.input(|i| {
        for e in &i.events {
            if let egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } = e {
                let k = physical_key.unwrap_or(*key);
                if k == Key::Escape {
                    return Recorded::Cancel;
                }
                if (k == Key::Backspace || k == Key::Delete) && !modifiers.any() {
                    return Recorded::Clear;
                }
                return Recorded::Set(Binding::from_event(k, *modifiers));
            }
        }
        Recorded::Nothing
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_matching() {
        let km = Keymap::default();
        let h = km.get(Action::Highlight).unwrap();
        assert!(h.matches(Key::H, Modifiers::NONE));
        assert!(!h.matches(Key::H, Modifiers::COMMAND));
        let next = km.get(Action::NextDoc).unwrap();
        assert!(next.matches(Key::CloseBracket, Modifiers::COMMAND | Modifiers::SHIFT));
        assert!(!next.matches(Key::CloseBracket, Modifiers::COMMAND));
    }

    #[test]
    fn rebinding_moves_a_taken_combination() {
        let mut km = Keymap::default();
        let h = km.get(Action::Highlight).unwrap();
        km.set(Action::Capture, Some(h.clone()));
        assert_eq!(km.get(Action::Capture), Some(h));
        assert_eq!(km.get(Action::Highlight), None);
        km.reset();
        assert!(km.get(Action::Highlight).is_some());
    }
}
