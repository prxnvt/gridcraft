//! Sequential ribbon shortcuts. The host paints hints from `prefix` and runs returned actions.

use egui::{Context, Event, Key, Modifiers, os::OperatingSystem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Show a ribbon tab (keytips stay on for its buttons).
    Tab(&'static str),
    Command(&'static str),
    Dialog(&'static str),
}

/// Fixed sequences: the tabs, and the Home sequences that predate per-button registration. Every
/// other button registers its own sequence while it is painted ([`KeyTips::register`]).
const FIXED: &[(&str, Action)] = &[
    ("H", Action::Tab("Home")),
    ("N", Action::Tab("Insert")),
    ("D", Action::Tab("Draw")),
    ("P", Action::Tab("Page Layout")),
    ("M", Action::Tab("Formulas")),
    ("A", Action::Tab("Data")),
    ("R", Action::Tab("Review")),
    ("W", Action::Tab("View")),
    ("Y", Action::Tab("Automate")),
    ("HAR", Action::Command("home.alignRight")),
    ("HAL", Action::Command("home.alignLeft")),
    ("HAC", Action::Command("home.alignCenter")),
    ("HVS", Action::Dialog("pasteSpecial")),
    ("ES", Action::Dialog("pasteSpecial")),
    ("HOI", Action::Command("home.autofitColumnWidth")),
    ("OCA", Action::Command("home.autofitColumnWidth")),
];

/// The keytip that shows a ribbon tab.
pub fn tab_key(tab: &str) -> Option<&'static str> {
    FIXED.iter().find(|(_, a)| matches!(a, Action::Tab(t) if *t == tab)).map(|(k, _)| *k)
}

#[derive(Default)]
pub struct KeyTips {
    prefix: Option<String>,
    alt_armed: bool,
    // Keep consuming a held mnemonic after its action closes keytips.
    held: Vec<Key>,
    /// Sequences registered by the buttons painted this frame.
    registered: Vec<String>,
    /// Sequences registered last frame: the ones a key press can complete now.
    known: Vec<String>,
    /// A registered sequence that was just typed; its button takes it when painted.
    activated: Option<String>,
}

impl KeyTips {
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    pub fn clear(&mut self) {
        self.prefix = None;
        self.alt_armed = false;
        self.held.clear();
    }

    /// Registers a button's sequence for this frame. Call it every frame the button is painted.
    pub fn register(&mut self, sequence: &str) {
        if !self.registered.iter().any(|s| s == sequence) {
            self.registered.push(sequence.to_string());
        }
    }

    /// The sequences buttons registered last frame: the ones keys can reach now.
    pub fn reachable(&self) -> &[String] {
        &self.known
    }

    /// Whether `sequence` was just typed; true once, for the button that registered it.
    pub fn take_activation(&mut self, sequence: &str) -> bool {
        if self.activated.as_deref() == Some(sequence) {
            self.activated = None;
            true
        } else {
            false
        }
    }

    /// Whether `sequence` is a known sequence's proper prefix (more letters are needed).
    fn is_prefix(&self, sequence: &str) -> bool {
        let longer = |s: &str| s.len() > sequence.len() && s.starts_with(sequence);
        FIXED.iter().any(|(s, _)| longer(s)) || self.known.iter().any(|s| longer(s))
    }

    fn toggle(&mut self) {
        self.prefix = if self.prefix.is_some() { None } else { Some(String::new()) };
        self.alt_armed = false;
    }

    /// Remove only handled events before any text editors or the grid see them.
    pub fn process(&mut self, ctx: &Context, enabled: bool) -> Vec<Action> {
        // Buttons register while painting, after this runs: what they registered last frame is
        // what the keys can reach now. An activation nobody took (its tab closed) expires.
        self.known = std::mem::take(&mut self.registered);
        self.activated = None;
        if !enabled || !ctx.input(|i| i.focused) {
            self.clear();
            return Vec::new();
        }
        let mac = ctx.os() == OperatingSystem::Mac;
        let events = ctx.input_mut(|i| std::mem::take(&mut i.events));
        let pairs = text_pairs(&events);
        let mut consumed = std::collections::HashSet::new();
        let mut actions = Vec::new();
        for (index, event) in events.iter().enumerate() {
            let mut handled = false;
            match event {
                Event::PointerButton { pressed: true, .. } | Event::MouseWheel { .. } | Event::WindowFocused(false) | Event::Ime(_) => self.clear(),
                Event::Copy | Event::Cut | Event::Paste(_) => self.clear(),
                Event::ModifiersChanged(m) => {
                    if command_modifier(*m) || (mac && m.alt) {
                        self.clear();
                    } else if m.shift {
                        self.alt_armed = false;
                    }
                }
                Event::Key { key: Key::AltLeft, pressed, repeat, modifiers, .. } if !mac && !command_modifier(*modifiers) && !modifiers.shift => {
                    if *pressed {
                        if !repeat {
                            self.alt_armed = true;
                        }
                        handled = true;
                    } else if self.alt_armed {
                        self.toggle();
                        handled = true;
                    }
                }
                Event::Key { key, pressed, repeat, modifiers, .. } => {
                    self.alt_armed = false;
                    if command_modifier(*modifiers)
                        || modifiers.alt
                        || matches!(key, Key::AltLeft | Key::AltRight | Key::ControlLeft | Key::ControlRight | Key::SuperLeft | Key::SuperRight)
                    {
                        self.clear();
                    } else if !pressed {
                        handled = self.held.contains(key);
                        self.held.retain(|held| held != key);
                    } else if *repeat && (self.prefix.is_some() || self.held.contains(key)) {
                        handled = true;
                    } else if *key == Key::F10 && modifiers.is_none() {
                        self.toggle();
                        handled = true;
                    } else if self.prefix.is_some() && !matches!(key, Key::ShiftLeft | Key::ShiftRight) {
                        handled = true;
                        if *key == Key::Escape {
                            if let Some(prefix) = self.prefix.as_mut()
                                && prefix.pop().is_none()
                            {
                                self.prefix = None;
                            }
                        } else if key.name().len() == 1 {
                            let next = format!("{}{key_name}", self.prefix.as_deref().unwrap_or_default(), key_name = key.name());
                            match FIXED.iter().find(|(s, _)| *s == next).map(|(_, a)| *a) {
                                Some(Action::Tab(tab)) => {
                                    self.prefix = Some(next);
                                    actions.push(Action::Tab(tab));
                                }
                                Some(action) => self.finish(action, &mut actions),
                                None if self.is_prefix(&next) => self.prefix = Some(next),
                                None if self.known.contains(&next) => {
                                    self.prefix = None;
                                    self.activated = Some(next);
                                }
                                None => {}
                            }
                        }
                    }
                    if handled && *pressed && !self.held.contains(key) {
                        self.held.push(*key);
                    }
                }
                Event::Text(_) => {
                    self.alt_armed = false;
                    // Paired text is decided with its key, which may occur later on web.
                    handled = self.prefix.is_some() && !pairs.iter().any(|(_, text)| *text == index);
                }
                _ => {}
            }
            if handled {
                consumed.insert(index);
                if let Some((_, text)) = pairs.iter().find(|(key, _)| *key == index) {
                    consumed.insert(*text);
                }
            }
        }
        ctx.input_mut(|i| {
            i.events = events.into_iter().enumerate().filter_map(|(index, event)| (!consumed.contains(&index)).then_some(event)).collect()
        });
        actions
    }

    fn finish(&mut self, action: Action, actions: &mut Vec<Action>) {
        self.prefix = None;
        actions.push(action);
    }
}

fn command_modifier(m: Modifiers) -> bool {
    m.ctrl || m.command || m.mac_cmd
}

/// Native integrations emit Key then Text; browsers can emit Text then Key.
fn text_pairs(events: &[Event]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for (text_index, event) in events.iter().enumerate() {
        let Event::Text(_) = event else { continue };
        let before = events.iter().enumerate().take(text_index).rev().find(|(_, e)| !matches!(e, Event::ModifiersChanged(_)));
        let after = events.iter().enumerate().skip(text_index + 1).find(|(_, e)| !matches!(e, Event::ModifiersChanged(_)));
        for (key_index, event) in before.into_iter().chain(after) {
            if let Event::Key { key, pressed: true, modifiers, .. } = event
                && (key.name().len() == 1 || modifiers.alt || command_modifier(*modifiers))
                && !pairs.iter().any(|(paired, _)| *paired == key_index)
            {
                pairs.push((key_index, text_index));
                break;
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
    }

    #[test]
    fn active_mode_preserves_modified_text_before_key() {
        for (os, letter, text, modifiers) in [
            (OperatingSystem::Windows, Key::E, "€", Modifiers::CTRL | Modifiers::ALT),
            (OperatingSystem::Nix, Key::E, "€", Modifiers::ALT),
            (OperatingSystem::Mac, Key::O, "œ", Modifiers::ALT),
            (OperatingSystem::Mac, Key::C, "c", Modifiers::COMMAND),
        ] {
            let ctx = Context::default();
            ctx.set_os(os);
            let mut tips = KeyTips { prefix: Some("H".into()), ..Default::default() };
            let events = vec![Event::Text(text.into()), key(letter, true, modifiers)];
            let mut remaining = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput { events: events.clone(), ..Default::default() }, |ui| {
                assert!(tips.process(ui.ctx(), true).is_empty());
                remaining = ui.input(|i| i.events.clone());
            });
            output.textures_delta.clear();
            assert_eq!(tips.prefix(), None);
            assert_eq!(remaining, events);
        }
    }

    #[test]
    fn shift_between_alt_press_and_release_does_not_activate() {
        let ctx = Context::default();
        ctx.set_os(OperatingSystem::Windows);
        let mut tips = KeyTips::default();
        let events =
            vec![key(Key::AltLeft, true, Modifiers::ALT), key(Key::ShiftLeft, true, Modifiers::NONE), key(Key::AltLeft, false, Modifiers::NONE)];
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            assert!(tips.process(ui.ctx(), true).is_empty());
        });
        output.textures_delta.clear();
        assert_eq!(tips.prefix(), None);
    }
}
