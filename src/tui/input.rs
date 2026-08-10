use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{Action, Geometry, Mode};

const NON_TEXT_MODIFIERS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

pub(super) fn map_event(mode: &Mode, terminal_too_small: bool, event: Event) -> Option<Action> {
    if let Event::Resize(width, height) = event {
        return Some(Action::Resize(Geometry::new(width, height)));
    }

    let Event::Key(key) = event else {
        return None;
    };
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
        return Some(Action::Interrupt);
    }
    if terminal_too_small {
        return (key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::NONE)
            .then_some(Action::Quit);
    }

    match mode {
        Mode::Reading => map_reading_key(key),
        Mode::SearchInput { .. } => map_search_key(key),
    }
}

fn map_reading_key(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => Some(Action::LineDown),
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => Some(Action::LineUp),
        (KeyCode::Char(' ') | KeyCode::PageDown, KeyModifiers::NONE)
        | (KeyCode::Char('f'), KeyModifiers::CONTROL) => Some(Action::PageDown),
        (KeyCode::Char('b') | KeyCode::PageUp, KeyModifiers::NONE)
        | (KeyCode::Char('b'), KeyModifiers::CONTROL) => Some(Action::PageUp),
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => Some(Action::HalfPageDown),
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Some(Action::HalfPageUp),
        (KeyCode::Char('g') | KeyCode::Home, KeyModifiers::NONE) => Some(Action::DocumentStart),
        (KeyCode::Char('G'), KeyModifiers::SHIFT) | (KeyCode::End, KeyModifiers::NONE) => {
            Some(Action::DocumentEnd)
        }
        (KeyCode::Char('/'), KeyModifiers::NONE) => Some(Action::BeginSearch),
        (KeyCode::Char('n'), KeyModifiers::NONE) => Some(Action::NextMatch),
        (KeyCode::Char('N'), KeyModifiers::SHIFT) => Some(Action::PreviousMatch),
        (KeyCode::Esc, KeyModifiers::NONE) => Some(Action::SearchCancel),
        (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Action::Quit),
        _ => None,
    }
}

fn map_search_key(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::Backspace, KeyModifiers::NONE) => Some(Action::SearchBackspace),
        (KeyCode::Enter, KeyModifiers::NONE) => Some(Action::SearchCommit),
        (KeyCode::Esc, KeyModifiers::NONE) => Some(Action::SearchCancel),
        (KeyCode::Char(character), modifiers) if !modifiers.intersects(NON_TEXT_MODIFIERS) => {
            Some(Action::SearchInsert(character))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyEventState, MouseEvent, MouseEventKind};

    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[test]
    fn reading_mode_maps_navigation_search_and_quit() {
        for (event, action) in [
            (
                key(KeyCode::Char('j'), KeyModifiers::NONE),
                Action::LineDown,
            ),
            (key(KeyCode::Up, KeyModifiers::NONE), Action::LineUp),
            (key(KeyCode::PageDown, KeyModifiers::NONE), Action::PageDown),
            (
                key(KeyCode::Char('b'), KeyModifiers::CONTROL),
                Action::PageUp,
            ),
            (
                key(KeyCode::Char('d'), KeyModifiers::CONTROL),
                Action::HalfPageDown,
            ),
            (
                key(KeyCode::Char('u'), KeyModifiers::CONTROL),
                Action::HalfPageUp,
            ),
            (
                key(KeyCode::Char('g'), KeyModifiers::NONE),
                Action::DocumentStart,
            ),
            (
                key(KeyCode::Char('G'), KeyModifiers::SHIFT),
                Action::DocumentEnd,
            ),
            (
                key(KeyCode::Char('/'), KeyModifiers::NONE),
                Action::BeginSearch,
            ),
            (
                key(KeyCode::Char('n'), KeyModifiers::NONE),
                Action::NextMatch,
            ),
            (
                key(KeyCode::Char('N'), KeyModifiers::SHIFT),
                Action::PreviousMatch,
            ),
            (key(KeyCode::Esc, KeyModifiers::NONE), Action::SearchCancel),
            (
                key(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Action::Interrupt,
            ),
            (key(KeyCode::Char('q'), KeyModifiers::NONE), Action::Quit),
        ] {
            assert_eq!(map_event(&Mode::Reading, false, event), Some(action));
        }
    }

    #[test]
    fn search_mode_accepts_text_editing_but_not_reading_commands() {
        let mode = Mode::SearchInput {
            draft: String::new(),
            limit_hit: false,
        };
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Action::SearchInsert('q'))
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Backspace, KeyModifiers::NONE)),
            Some(Action::SearchBackspace)
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Action::SearchCommit)
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::SearchCancel)
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn resize_and_tiny_terminal_rules_are_mode_aware() {
        assert_eq!(
            map_event(&Mode::Reading, true, Event::Resize(80, 24)),
            Some(Action::Resize(Geometry::new(80, 24)))
        );
        assert_eq!(
            map_event(
                &Mode::Reading,
                true,
                key(KeyCode::Char('j'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            map_event(
                &Mode::Reading,
                true,
                key(KeyCode::Char('q'), KeyModifiers::NONE)
            ),
            Some(Action::Quit)
        );
        let search = Mode::SearchInput {
            draft: String::new(),
            limit_hit: false,
        };
        assert_eq!(
            map_event(&search, true, key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Action::Quit)
        );
        assert_eq!(
            map_event(
                &search,
                true,
                key(KeyCode::Char('c'), KeyModifiers::CONTROL)
            ),
            Some(Action::Interrupt)
        );
    }

    #[test]
    fn release_and_nonkeyboard_events_are_ignored() {
        let release = Event::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
        assert_eq!(map_event(&Mode::Reading, false, release), None);
        assert_eq!(
            map_event(
                &Mode::Reading,
                false,
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Moved,
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                })
            ),
            None
        );
    }
}
