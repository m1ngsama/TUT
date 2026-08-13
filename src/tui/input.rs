use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use crate::app::{Action, ContentMode, Geometry, Mode};

const NON_TEXT_MODIFIERS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

fn normalize_key_case(mut key: KeyEvent) -> KeyEvent {
    let KeyCode::Char(character) = key.code else {
        return key;
    };
    if !character.is_ascii_alphabetic() {
        return key;
    }

    // Legacy and original CSI-u input contain the produced character, while Kitty-style CSI-u's
    // primary codepoint is the unshifted key. Crossterm can also consume SHIFT when a Kitty
    // alternate keycode supplies the produced character. Treat an uppercase codepoint as the
    // consumed SHIFT representation, then apply Caps Lock only to text-like keys. Lock state must
    // not change physical Ctrl/Alt shortcut matching (for example, Ctrl-C remains Ctrl-C while
    // Caps Lock is on).
    let shifted = character.is_ascii_uppercase() || key.modifiers.contains(KeyModifiers::SHIFT);
    let uppercase = shifted
        ^ (key.state.contains(KeyEventState::CAPS_LOCK)
            && !key.modifiers.intersects(NON_TEXT_MODIFIERS));
    key.code = KeyCode::Char(if uppercase {
        character.to_ascii_uppercase()
    } else {
        character.to_ascii_lowercase()
    });
    key.modifiers.set(KeyModifiers::SHIFT, uppercase);
    key
}

pub(super) fn map_event(
    mode: &Mode,
    repeat_active: bool,
    terminal_too_small: bool,
    event: Event,
) -> Option<Action> {
    if let Event::Resize(width, height) = event {
        return Some(Action::Resize(Geometry::new(width, height)));
    }

    let Event::Key(key) = event else {
        return None;
    };
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    // Crossterm treats `Char('g') + SHIFT` and `Char('G') + SHIFT` as the same key,
    // but different terminal protocols can emit either representation. Canonicalize before
    // matching commands or inserting search text so their behavior agrees as well.
    let key = normalize_key_case(key);
    if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
        return Some(Action::Interrupt);
    }
    if terminal_too_small {
        return (key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('q')
            && key.modifiers == KeyModifiers::NONE)
            .then_some(Action::Quit);
    }

    match mode {
        Mode::Content(ContentMode::Reading) => map_reading_key(key, repeat_active),
        Mode::Content(ContentMode::SearchInput { .. }) => map_search_key(key),
        Mode::Help { return_to } => map_help_key(return_to, key),
    }
}

fn map_reading_key(key: KeyEvent, repeat_active: bool) -> Option<Action> {
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
        (KeyCode::Char(digit @ '0'..='9'), KeyModifiers::NONE)
            if key.kind == KeyEventKind::Press && (digit != '0' || repeat_active) =>
        {
            Some(Action::RepeatDigit(
                digit
                    .to_digit(10)
                    .expect("ASCII digits have decimal values") as u8,
            ))
        }
        (KeyCode::Char('0'..='9'), KeyModifiers::NONE) => None,
        (KeyCode::Backspace, KeyModifiers::NONE) if repeat_active => Some(Action::RepeatBackspace),
        (KeyCode::F(1), KeyModifiers::NONE) if key.kind == KeyEventKind::Press => {
            Some(Action::ShowHelp)
        }
        (KeyCode::Esc, KeyModifiers::NONE) if key.kind == KeyEventKind::Press && repeat_active => {
            Some(Action::RepeatCancel)
        }
        (KeyCode::Esc, KeyModifiers::NONE) if key.kind == KeyEventKind::Press => {
            Some(Action::SearchCancel)
        }
        (KeyCode::Char('q'), KeyModifiers::NONE) if key.kind == KeyEventKind::Press => {
            Some(Action::Quit)
        }
        // Kitty can report modifier and lock keys separately. They change the following chord
        // rather than acting as unknown commands, so they must not discard an active repeat
        // prefix.
        (KeyCode::Modifier(_) | KeyCode::CapsLock | KeyCode::ScrollLock | KeyCode::NumLock, _) => {
            None
        }
        _ if key.kind == KeyEventKind::Press && repeat_active => Some(Action::RepeatCancel),
        _ => None,
    }
}

fn map_help_key(return_to: &ContentMode, key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::F(1) | KeyCode::Esc, KeyModifiers::NONE) if key.kind == KeyEventKind::Press => {
            Some(Action::DismissHelp)
        }
        (KeyCode::Char('q'), KeyModifiers::NONE)
            if key.kind == KeyEventKind::Press && matches!(return_to, ContentMode::Reading) =>
        {
            Some(Action::DismissHelp)
        }
        _ => None,
    }
}

fn map_search_key(key: KeyEvent) -> Option<Action> {
    match (key.code, key.modifiers) {
        (KeyCode::F(1), KeyModifiers::NONE) if key.kind == KeyEventKind::Press => {
            Some(Action::ShowHelp)
        }
        (KeyCode::Backspace, KeyModifiers::NONE) => Some(Action::SearchBackspace),
        (KeyCode::Up, KeyModifiers::NONE) if key.kind == KeyEventKind::Press => {
            Some(Action::SearchRecall)
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) if key.kind == KeyEventKind::Press => {
            Some(Action::SearchClearDraft)
        }
        (KeyCode::Enter, KeyModifiers::NONE) => Some(Action::SearchCommit),
        (KeyCode::Esc, KeyModifiers::NONE) if key.kind == KeyEventKind::Press => {
            Some(Action::SearchCancel)
        }
        (KeyCode::Char(character), modifiers) if !modifiers.intersects(NON_TEXT_MODIFIERS) => {
            Some(Action::SearchInsert(character))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{ModifierKeyCode, MouseEvent, MouseEventKind};

    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn repeated_key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        key_with_kind_and_state(code, modifiers, KeyEventKind::Repeat, KeyEventState::NONE)
    }

    fn key_with_state(code: KeyCode, modifiers: KeyModifiers, state: KeyEventState) -> Event {
        key_with_kind_and_state(code, modifiers, KeyEventKind::Press, state)
    }

    fn key_with_kind_and_state(
        code: KeyCode,
        modifiers: KeyModifiers,
        kind: KeyEventKind,
        state: KeyEventState,
    ) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            state,
        })
    }

    fn map_event(mode: &Mode, terminal_too_small: bool, event: Event) -> Option<Action> {
        super::map_event(mode, false, terminal_too_small, event)
    }

    const fn reading_mode() -> Mode {
        Mode::Content(ContentMode::Reading)
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
            (key(KeyCode::F(1), KeyModifiers::NONE), Action::ShowHelp),
            (key(KeyCode::Esc, KeyModifiers::NONE), Action::SearchCancel),
            (
                key(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Action::Interrupt,
            ),
            (key(KeyCode::Char('q'), KeyModifiers::NONE), Action::Quit),
        ] {
            assert_eq!(map_event(&reading_mode(), false, event), Some(action));
        }
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key(KeyCode::Char('?'), KeyModifiers::SHIFT)
            ),
            None
        );
    }

    #[test]
    fn reader_repeat_prefix_accepts_only_pressed_digits_and_cancels_unknown_keys() {
        let mode = reading_mode();
        let map = |active, event| super::map_event(&mode, active, false, event);

        assert_eq!(
            map(false, key(KeyCode::Char('1'), KeyModifiers::NONE)),
            Some(Action::RepeatDigit(1))
        );
        assert_eq!(
            map(false, key(KeyCode::Char('0'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            map(true, key(KeyCode::Char('0'), KeyModifiers::NONE)),
            Some(Action::RepeatDigit(0))
        );
        assert_eq!(
            map(true, repeated_key(KeyCode::Char('2'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            map(true, repeated_key(KeyCode::Char('j'), KeyModifiers::NONE)),
            Some(Action::LineDown)
        );
        assert_eq!(
            map(true, key(KeyCode::Backspace, KeyModifiers::NONE)),
            Some(Action::RepeatBackspace)
        );
        assert_eq!(
            map(true, key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::RepeatCancel)
        );
        assert_eq!(
            map(true, key(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            Some(Action::RepeatCancel)
        );
        for (code, modifiers) in [
            (ModifierKeyCode::LeftShift, KeyModifiers::SHIFT),
            (ModifierKeyCode::RightControl, KeyModifiers::CONTROL),
            (ModifierKeyCode::LeftAlt, KeyModifiers::ALT),
            (ModifierKeyCode::RightSuper, KeyModifiers::SUPER),
            (ModifierKeyCode::LeftHyper, KeyModifiers::HYPER),
            (ModifierKeyCode::RightMeta, KeyModifiers::META),
            (ModifierKeyCode::IsoLevel3Shift, KeyModifiers::NONE),
            (ModifierKeyCode::IsoLevel5Shift, KeyModifiers::NONE),
        ] {
            assert_eq!(map(true, key(KeyCode::Modifier(code), modifiers)), None);
        }
        for code in [KeyCode::CapsLock, KeyCode::ScrollLock, KeyCode::NumLock] {
            assert_eq!(map(true, key(code, KeyModifiers::NONE)), None);
        }
        assert_eq!(
            map(true, key(KeyCode::Char('g'), KeyModifiers::NONE)),
            Some(Action::DocumentStart)
        );
        assert_eq!(
            map(true, key(KeyCode::Char('/'), KeyModifiers::NONE)),
            Some(Action::BeginSearch)
        );
        assert_eq!(
            map(true, key(KeyCode::F(1), KeyModifiers::NONE)),
            Some(Action::ShowHelp)
        );
        assert_eq!(
            map(true, key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Action::Quit)
        );
        assert_eq!(
            map(true, key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Action::Interrupt)
        );
    }

    #[test]
    fn search_mode_accepts_text_editing_but_not_reading_commands() {
        let mode = Mode::Content(ContentMode::SearchInput {
            draft: String::new(),
            limit_hit: false,
        });
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Action::SearchInsert('q'))
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            Some(Action::SearchInsert('?'))
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Char('7'), KeyModifiers::NONE)),
            Some(Action::SearchInsert('7'))
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Backspace, KeyModifiers::NONE)),
            Some(Action::SearchBackspace)
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Up, KeyModifiers::NONE)),
            Some(Action::SearchRecall)
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            Some(Action::SearchClearDraft)
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Action::SearchCommit)
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::F(1), KeyModifiers::NONE)),
            Some(Action::ShowHelp)
        );
        assert_eq!(
            map_event(&mode, false, key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(Action::SearchCancel)
        );
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::HYPER,
            KeyModifiers::META,
            KeyModifiers::SHIFT | KeyModifiers::ALT,
        ] {
            assert_eq!(
                map_event(&mode, false, key(KeyCode::Char('x'), modifiers)),
                None
            );
        }
        assert_eq!(
            map_event(&mode, false, repeated_key(KeyCode::Up, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            map_event(
                &mode,
                false,
                repeated_key(KeyCode::Char('u'), KeyModifiers::CONTROL)
            ),
            None
        );
    }

    #[test]
    fn csi_u_case_representations_are_normalized_before_mapping() {
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key(KeyCode::Char('g'), KeyModifiers::SHIFT)
            ),
            Some(Action::DocumentEnd)
        );
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key(KeyCode::Char('N'), KeyModifiers::NONE)
            ),
            Some(Action::PreviousMatch)
        );
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key(KeyCode::Char('q'), KeyModifiers::SHIFT)
            ),
            None
        );

        let search = Mode::Content(ContentMode::SearchInput {
            draft: String::new(),
            limit_hit: false,
        });
        assert_eq!(
            map_event(&search, false, key(KeyCode::Char('a'), KeyModifiers::SHIFT)),
            Some(Action::SearchInsert('A'))
        );
    }

    #[test]
    fn reader_commands_and_interrupt_require_exact_physical_modifiers() {
        for modifiers in [
            KeyModifiers::ALT,
            KeyModifiers::CONTROL,
            KeyModifiers::SUPER,
            KeyModifiers::HYPER,
            KeyModifiers::META,
            KeyModifiers::SHIFT | KeyModifiers::ALT,
            KeyModifiers::SHIFT | KeyModifiers::CONTROL,
        ] {
            for character in ['g', 'n', 'q'] {
                assert_eq!(
                    map_event(
                        &reading_mode(),
                        false,
                        key(KeyCode::Char(character), modifiers),
                    ),
                    None
                );
            }
        }

        for modifiers in [
            KeyModifiers::NONE,
            KeyModifiers::SHIFT,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::HYPER,
            KeyModifiers::META,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            assert_eq!(
                map_event(&reading_mode(), false, key(KeyCode::Char('c'), modifiers),),
                None
            );
        }
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            ),
            Some(Action::Interrupt)
        );
    }

    #[test]
    fn kitty_caps_lock_state_preserves_text_case_without_changing_control_shortcuts() {
        let caps_lock = KeyEventState::CAPS_LOCK;
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key_with_state(KeyCode::Char('g'), KeyModifiers::NONE, caps_lock),
            ),
            Some(Action::DocumentEnd)
        );
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key_with_state(KeyCode::Char('n'), KeyModifiers::NONE, caps_lock),
            ),
            Some(Action::PreviousMatch)
        );
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key_with_state(KeyCode::Char('q'), KeyModifiers::NONE, caps_lock),
            ),
            None
        );
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key_with_state(KeyCode::Char('g'), KeyModifiers::SHIFT, caps_lock),
            ),
            Some(Action::DocumentStart)
        );
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                key_with_state(KeyCode::Char('q'), KeyModifiers::SHIFT, caps_lock),
            ),
            Some(Action::Quit)
        );

        for (modifiers, expected) in [
            (KeyModifiers::CONTROL, Some(Action::Interrupt)),
            (KeyModifiers::CONTROL | KeyModifiers::SHIFT, None),
        ] {
            assert_eq!(
                map_event(
                    &reading_mode(),
                    false,
                    key_with_state(KeyCode::Char('c'), modifiers, caps_lock),
                ),
                expected
            );
        }

        let search = Mode::Content(ContentMode::SearchInput {
            draft: String::new(),
            limit_hit: false,
        });
        assert_eq!(
            map_event(
                &search,
                false,
                key_with_state(KeyCode::Char('a'), KeyModifiers::NONE, caps_lock),
            ),
            Some(Action::SearchInsert('A'))
        );
        assert_eq!(
            map_event(
                &search,
                false,
                key_with_state(KeyCode::Char('a'), KeyModifiers::SHIFT, caps_lock),
            ),
            Some(Action::SearchInsert('a'))
        );
    }

    #[test]
    fn help_mode_maps_dismissal_but_not_reader_commands() {
        for event in [
            key(KeyCode::F(1), KeyModifiers::NONE),
            key(KeyCode::Esc, KeyModifiers::NONE),
            key(KeyCode::Char('q'), KeyModifiers::NONE),
        ] {
            assert_eq!(
                map_event(
                    &Mode::Help {
                        return_to: ContentMode::Reading,
                    },
                    false,
                    event,
                ),
                Some(Action::DismissHelp)
            );
        }
        assert_eq!(
            map_event(
                &Mode::Help {
                    return_to: ContentMode::Reading,
                },
                false,
                key(KeyCode::Char('j'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            super::map_event(
                &Mode::Help {
                    return_to: ContentMode::Reading,
                },
                true,
                false,
                key(KeyCode::Char('7'), KeyModifiers::NONE),
            ),
            None
        );
        assert_eq!(
            map_event(
                &Mode::Help {
                    return_to: ContentMode::SearchInput {
                        draft: "draft".to_owned(),
                        limit_hit: false,
                    },
                },
                false,
                repeated_key(KeyCode::Char('q'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            map_event(
                &Mode::Help {
                    return_to: ContentMode::SearchInput {
                        draft: "draft".to_owned(),
                        limit_hit: false,
                    },
                },
                false,
                key(KeyCode::Char('q'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            map_event(
                &Mode::Help {
                    return_to: ContentMode::Reading,
                },
                false,
                key(KeyCode::Char('?'), KeyModifiers::SHIFT)
            ),
            None
        );
        assert_eq!(
            map_event(
                &Mode::Help {
                    return_to: ContentMode::Reading,
                },
                false,
                key(KeyCode::Char('c'), KeyModifiers::CONTROL)
            ),
            Some(Action::Interrupt)
        );
    }

    #[test]
    fn resize_and_tiny_terminal_rules_are_mode_aware() {
        assert_eq!(
            map_event(&reading_mode(), true, Event::Resize(80, 24)),
            Some(Action::Resize(Geometry::new(80, 24)))
        );
        assert_eq!(
            map_event(
                &reading_mode(),
                true,
                key(KeyCode::Char('j'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            map_event(
                &reading_mode(),
                true,
                key(KeyCode::Char('q'), KeyModifiers::NONE)
            ),
            Some(Action::Quit)
        );
        assert_eq!(
            map_event(
                &reading_mode(),
                true,
                repeated_key(KeyCode::Char('q'), KeyModifiers::NONE)
            ),
            None
        );
        let search = Mode::Content(ContentMode::SearchInput {
            draft: String::new(),
            limit_hit: false,
        });
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

        let help = Mode::Help {
            return_to: ContentMode::Reading,
        };
        assert_eq!(
            map_event(&help, true, key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Action::Quit)
        );
        for code in [KeyCode::F(1), KeyCode::Esc] {
            assert_eq!(map_event(&help, true, key(code, KeyModifiers::NONE)), None);
        }
    }

    #[test]
    fn release_and_nonkeyboard_events_are_ignored() {
        let release = Event::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
        assert_eq!(map_event(&reading_mode(), false, release), None);
        assert_eq!(
            map_event(
                &reading_mode(),
                false,
                repeated_key(KeyCode::Char('j'), KeyModifiers::NONE)
            ),
            Some(Action::LineDown)
        );
        for code in [KeyCode::F(1), KeyCode::Esc, KeyCode::Char('q')] {
            assert_eq!(
                map_event(
                    &reading_mode(),
                    false,
                    repeated_key(code, KeyModifiers::NONE)
                ),
                None
            );
        }
        for code in [KeyCode::F(1), KeyCode::Esc, KeyCode::Char('q')] {
            assert_eq!(
                map_event(
                    &Mode::Help {
                        return_to: ContentMode::Reading,
                    },
                    false,
                    repeated_key(code, KeyModifiers::NONE),
                ),
                None
            );
        }
        assert_eq!(
            map_event(
                &reading_mode(),
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
