//! What the window's close button does.
//!
//! Three answers rather than two, because there are three things that can be true at once: somebody
//! may have asked to quit, the application may be set to go away when the window is closed, and the
//! window may be about to be put somewhere it can be picked up again. Which one wins is the whole of
//! this module, and it is a function of its own so that the answer can be checked without a window —
//! the shell around it can only be tested by hand.

/// What a close request turns into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Put the window away and keep running: the application is not set to quit, and there is an icon
    /// to bring it back from.
    Hide,
    /// Ask the interface first, because it said there is something to lose.
    Ask,
    /// Let the window go. The application ends with it.
    Close,
}

/// Which of the three a close request is.
///
/// The order is the argument. A close somebody has already answered is final — asking twice is how a
/// window ends up impossible to shut. Otherwise a window that hides hides: hiding loses nothing, so
/// there is nothing to warn about, and the question belongs to the case where the work really goes
/// away. The one case that is *not* covered here is a window set to hide with no icon to hide to:
/// that combination would leave a process running with no way back to it, so it closes instead, which
/// is also what this application did before it had a tray at all.
pub fn close_action(approved: bool, quit_on_close: bool, tray_shown: bool, guard: bool) -> CloseAction {
    if approved {
        return CloseAction::Close;
    }
    if !quit_on_close && tray_shown {
        return CloseAction::Hide;
    }
    if guard {
        return CloseAction::Ask;
    }
    CloseAction::Close
}

#[cfg(test)]
mod tests {
    use super::{CloseAction, close_action};

    #[test]
    fn a_close_that_was_answered_is_not_asked_again() {
        // Whatever else is true: an approved close is a close.
        for quit_on_close in [true, false] {
            for tray_shown in [true, false] {
                for guard in [true, false] {
                    assert_eq!(
                        close_action(true, quit_on_close, tray_shown, guard),
                        CloseAction::Close,
                        "the interface answered and the window refused to go"
                    );
                }
            }
        }
    }

    #[test]
    fn a_window_set_to_put_itself_away_hides_without_asking() {
        // Hiding throws nothing away, so the question would be about nothing — and the guard being
        // armed is not a reason to ask when nothing is going to be lost.
        assert_eq!(close_action(false, false, true, false), CloseAction::Hide);
        assert_eq!(close_action(false, false, true, true), CloseAction::Hide);
    }

    #[test]
    fn quitting_on_close_asks_when_there_is_something_to_lose() {
        assert_eq!(close_action(false, true, true, true), CloseAction::Ask);
        assert_eq!(close_action(false, true, false, true), CloseAction::Ask);
        assert_eq!(close_action(false, true, true, false), CloseAction::Close);
    }

    #[test]
    fn a_window_with_nothing_to_hide_to_closes_instead_of_hiding() {
        // The icon off and "put the window away" on: hiding would leave a process nobody can reach,
        // so this pair closes — which is what the shell did before it had a tray.
        assert_eq!(close_action(false, false, false, false), CloseAction::Close);
        assert_eq!(close_action(false, false, false, true), CloseAction::Ask);
    }
}
