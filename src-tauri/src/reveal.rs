//! Handing a path to the system's file manager, so somebody can see where a file actually is.
//!
//! It lives on this side of the boundary because of what it needs: a path. A transfer's local end
//! was chosen in a native dialog and has never crossed back through the interface, so what the
//! interface sends is the transfer's id and the path is looked up here. A build that let the page
//! name a path would be a build where anything running in the webview could ask the system to open
//! whatever it liked.
//!
//! The three systems are asked three ways, and the ways differ in more than the program's name:
//! Windows marks a file through a single `/select,<path>` argument, macOS has a flag for it, and a
//! Linux desktop has no way to say "mark this one" at all. Each shape is held to a test rather than
//! to the memory of whoever last used that system, because nobody can check the other two by
//! reading their own.

use std::path::Path;

use sevenshell_error::ShellError;

/// What the system is asked to do with a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    /// Open it: a file with whatever application is registered for it, a folder in the file manager.
    Open,
    /// Show where it is, with the file itself marked when the platform can mark one.
    Select,
}

/// Which system's way of being asked is used.
///
/// A parameter rather than a compile-time choice so that all three ways are checked by the same test
/// on whichever machine the tests happen to run on. That is also why all three variants exist in
/// every build: `current()` produces one of them, and the test drives the other two.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    /// The one this build is running on.
    pub fn current() -> Self {
        #[cfg(target_os = "windows")]
        return Platform::Windows;
        #[cfg(target_os = "macos")]
        return Platform::MacOs;
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        return Platform::Linux;
    }
}

/// The program to run, and what to run it with.
///
/// `exists` decides one case, and it is not a detail: asked to mark a file that is not there,
/// Windows' explorer opens Documents instead — a window onto somewhere nobody asked about — so a
/// path that is not there is shown by the folder that holds it. That folder is also where the part
/// file of a transfer that stopped would be found.
pub fn command_for(platform: Platform, path: &Path, show: Show, exists: bool) -> (String, Vec<String>) {
    let shown = path.display().to_string();
    let holder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.display().to_string())
        .unwrap_or_else(|| shown.clone());

    match platform {
        Platform::Windows => match (show, exists) {
            (Show::Select, true) => ("explorer".to_owned(), vec![format!("/select,{shown}")]),
            (Show::Select, false) => ("explorer".to_owned(), vec![holder]),
            // `start` is the system's "open this with its application", and the empty argument is
            // what keeps a quoted path from being read as the window title.
            (Show::Open, _) => (
                "cmd".to_owned(),
                vec!["/c".to_owned(), "start".to_owned(), String::new(), shown],
            ),
        },
        Platform::MacOs => (
            "open".to_owned(),
            match show {
                Show::Select => vec!["-R".to_owned(), shown],
                Show::Open => vec![shown],
            },
        ),
        Platform::Linux => (
            "xdg-open".to_owned(),
            vec![match show {
                Show::Select => holder,
                Show::Open => shown,
            }],
        ),
    }
}

/// Asks the system to show the path, and does not wait for the answer.
///
/// Not waited for because the file manager is not this application's child: waiting would hold the
/// caller — a command the interface is standing on — for as long as somebody leaves that window
/// open.
pub fn show(path: &Path, what: Show) -> Result<(), ShellError> {
    let (program, args) = command_for(Platform::current(), path, what, path.exists());
    let mut command = std::process::Command::new(&program);
    command.args(&args);

    // A console window flashing up behind the file manager is not something to leave in a desktop
    // application, and this is the only place one could appear.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| ShellError::fs_open_failed(format!("{program} could not be run: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    #[test]
    fn windows_marks_a_file_through_the_single_argument_explorer_wants() {
        assert_eq!(
            command_for(Platform::Windows, &path(r"C:\Users\me\a b.txt"), Show::Select, true),
            ("explorer".to_owned(), vec![r"/select,C:\Users\me\a b.txt".to_owned()])
        );
    }

    #[test]
    fn windows_opens_through_start_with_the_argument_that_keeps_a_path_from_being_a_title() {
        let (program, args) = command_for(Platform::Windows, &path(r"C:\Users\me\a b.txt"), Show::Open, true);

        assert_eq!(program, "cmd");
        assert_eq!(
            args,
            vec![
                "/c".to_owned(),
                "start".to_owned(),
                String::new(),
                r"C:\Users\me\a b.txt".to_owned()
            ]
        );
    }

    #[test]
    fn a_path_that_is_not_there_is_shown_by_the_folder_holding_it() {
        // Explorer asked to mark a path that does not exist opens Documents, and "where the part
        // file is" is the folder anyway.
        //
        // The folder holding a path is worked out with the host's own path rules, the application
        // never being handed a path written for another platform, so a path in Windows' syntax can
        // only be asserted on Windows: on macOS and Linux a `\` is an ordinary character, and
        // `C:\dl\half.bin` is one name with no folder in front of it.
        #[cfg(windows)]
        assert_eq!(
            command_for(Platform::Windows, &path(r"C:\dl\half.bin"), Show::Select, false).1,
            vec![r"C:\dl".to_owned()]
        );
        assert_eq!(
            command_for(Platform::Linux, &path("/home/me/half.bin"), Show::Select, true).1,
            vec!["/home/me".to_owned()]
        );
    }

    #[test]
    fn macos_marks_with_the_flag_that_makes_it_a_mark() {
        let (program, args) = command_for(Platform::MacOs, &path("/Users/me/a b.txt"), Show::Select, true);

        assert_eq!(program, "open");
        assert_eq!(args, vec!["-R".to_owned(), "/Users/me/a b.txt".to_owned()]);
    }

    #[test]
    fn a_path_with_no_folder_of_its_own_is_shown_as_itself() {
        // The root of a drive has no parent, and asking to show "" is asking for somewhere that is
        // not anywhere at all.
        assert_eq!(
            command_for(Platform::Linux, &path("/"), Show::Select, false).1,
            vec!["/".to_owned()]
        );
    }
}
