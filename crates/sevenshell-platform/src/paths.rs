//! Where the application keeps its data.
//!
//! Hand-rolled rather than delegated to a helper crate: there are three short rules, and the
//! directory has to match what the desktop shell already uses, which is
//! `{configuration directory}/{identifier}` on every platform.
//!
//! `target_os` is a parameter rather than `std::env::consts::OS` so that the rules for all
//! three platforms can be tested from any one of them.

use std::path::PathBuf;

/// The application identifier. Kept in step with `src-tauri/tauri.conf.json` by
/// `scripts/tests/architecture.test.mjs`'s sibling check in the interface tests.
pub const APP_IDENTIFIER: &str = "com.sevenshell.app";

/// The settings file name inside the configuration directory.
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// Resolves the configuration directory for one platform, reading the environment through
/// the supplied lookup so tests can describe a machine they are not running on.
pub fn config_dir_for(target_os: &str, environment: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    match target_os {
        "windows" => environment("APPDATA").map(|base| PathBuf::from(base).join(APP_IDENTIFIER)),
        "macos" => environment("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(APP_IDENTIFIER)
        }),
        _ => environment("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| environment("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|base| base.join(APP_IDENTIFIER)),
    }
}

/// The configuration directory of the machine we are running on.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_for(std::env::consts::OS, &|key| std::env::var(key).ok())
}

/// Where the settings file lives, whether or not it exists yet.
pub fn settings_file() -> Option<PathBuf> {
    config_dir().map(|directory| directory.join(SETTINGS_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn windows_uses_appdata_and_the_application_identifier() {
        let env = environment(&[("APPDATA", "C:\\Users\\someone\\AppData\\Roaming")]);
        let directory = config_dir_for("windows", &env).expect("APPDATA is set");

        assert_eq!(
            directory,
            PathBuf::from("C:\\Users\\someone\\AppData\\Roaming").join(APP_IDENTIFIER)
        );
    }

    #[test]
    fn macos_uses_application_support() {
        let env = environment(&[("HOME", "/Users/someone")]);
        let directory = config_dir_for("macos", &env).expect("HOME is set");

        assert_eq!(
            directory,
            PathBuf::from("/Users/someone/Library/Application Support").join(APP_IDENTIFIER)
        );
    }

    #[test]
    fn linux_prefers_xdg_config_home_and_falls_back_to_dot_config() {
        let with_xdg = environment(&[
            ("XDG_CONFIG_HOME", "/home/someone/.config-custom"),
            ("HOME", "/home/someone"),
        ]);
        assert_eq!(
            config_dir_for("linux", &with_xdg).expect("XDG_CONFIG_HOME is set"),
            PathBuf::from("/home/someone/.config-custom").join(APP_IDENTIFIER)
        );

        let without_xdg = environment(&[("HOME", "/home/someone")]);
        assert_eq!(
            config_dir_for("linux", &without_xdg).expect("HOME is set"),
            PathBuf::from("/home/someone/.config").join(APP_IDENTIFIER)
        );
    }

    #[test]
    fn a_missing_environment_variable_is_not_a_panic() {
        let empty = environment(&[]);

        assert!(config_dir_for("windows", &empty).is_none());
        assert!(config_dir_for("macos", &empty).is_none());
        assert!(config_dir_for("linux", &empty).is_none());
    }
}
