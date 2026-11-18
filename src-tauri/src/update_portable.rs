//! In-place updates for a portable copy.
//!
//! A portable copy has no installation for the Windows installer to find or replace: asking the
//! built-in update to install would put a second copy under `%LOCALAPPDATA%` and leave the running
//! one — its vault, its settings, its shortcut — exactly where it was. So this copy replaces
//! itself: the signed portable archive is downloaded, unpacked beside the executable, the running
//! executable is renamed out of the way (Windows allows that; the image stays mapped under the old
//! name), the new one is written at the path the process was started from, and the caller restarts
//! into it. The `data` folder beside the executable never moves, so what the copy *is* does not
//! change.

use std::path::Path;

use sevenshell_error::ShellError;
use tauri_plugin_updater::Update;

/// The archive a portable release carries, and the two files this copy takes from it.
///
/// The rest of the archive (the licence and the readmes) is written by the release for people who
/// unpack it by hand; an update has no reason to rewrite files the running copy does not read.
const ARCHIVE_EXE: &str = "7Shell.exe";
const ARCHIVE_SEED: &str = "sshell-mcp-seed.exe";

/// Where the archive is unpacked, beside the executable so the move is a rename on one volume.
const STAGING_DIR: &str = ".7shell-update";

/// What a replaced executable is called until the next start can delete it: the old process still
/// has the file open while the new copy is being written, so it cannot be removed yet.
const RETIRED_SUFFIX: &str = ".retired";

/// One archive the manifest names for a portable copy.
#[derive(Clone)]
pub struct PortableSource {
    url: url::Url,
    signature: String,
}

/// Whether the running executable is a portable copy.
pub fn this_copy_is_portable() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| sevenshell_platform::paths::portable_data_dir_for(&exe))
        .is_some()
}

/// The entry a portable copy replaces itself from, when the manifest names one.
pub fn portable_source(update: &Update) -> Option<PortableSource> {
    portable_entry(&update.raw_json, &update.target)
}

/// The same release, redirected at the portable archive.
///
/// The download keeps the updater's client and its signature check — the archive is signed by the
/// key the build already carries, and handing these bytes to anything this project wrote itself
/// would be a second verifier to keep honest.
pub fn archive_for(update: &Update, source: &PortableSource) -> Update {
    let mut archive = update.clone();
    archive.download_url = source.url.clone();
    archive.signature = source.signature.clone();
    archive
}

/// Replaces the running copy with `archive`, which must carry [`ARCHIVE_EXE`].
pub fn install(archive: &[u8]) -> Result<(), ShellError> {
    let exe = std::env::current_exe()
        .map_err(|error| failure(format!("the running executable could not be located: {error}")))?;
    apply(&exe, archive)
}

/// Removes what an interrupted update left beside the executable.
///
/// Called on every start of a portable copy: a retired executable and a staging folder are only
/// deletable after the process that held them is gone, which is exactly the next start.
pub fn clean_leftovers() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if sevenshell_platform::paths::portable_data_dir_for(&exe).is_none() {
        return;
    }
    let Some(directory) = exe.parent() else {
        return;
    };

    if let Some(name) = exe.file_name() {
        let retired = directory.join(format!("{}{RETIRED_SUFFIX}", name.to_string_lossy()));
        if let Err(error) = std::fs::remove_file(&retired)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("the copy an update replaced could not be removed: {error}");
        }
    }

    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy() == STAGING_DIR && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The manifest entry, read as JSON rather than through a type: the plugin's own release struct
/// keeps only the entry it picked, and this is the other entry of the same manifest.
fn portable_entry(manifest: &serde_json::Value, target: &str) -> Option<PortableSource> {
    let entry = manifest.get("platforms")?.get(format!("{target}-portable"))?;
    let url = url::Url::parse(entry.get("url")?.as_str()?).ok()?;
    let signature = entry.get("signature")?.as_str()?.to_owned();
    Some(PortableSource { url, signature })
}

fn apply(exe: &Path, archive: &[u8]) -> Result<(), ShellError> {
    let directory = exe
        .parent()
        .ok_or_else(|| failure("the running executable has no directory"))?;
    let staging = directory.join(STAGING_DIR);
    let _ = std::fs::remove_dir_all(&staging);
    extract(archive, &staging)?;

    let staged_exe = staging.join(ARCHIVE_EXE);
    if !staged_exe.is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(failure(format!("the archive holds no {ARCHIVE_EXE}")));
    }

    let name = exe
        .file_name()
        .ok_or_else(|| failure("the running executable has no name"))?;
    let retired = directory.join(format!("{}{RETIRED_SUFFIX}", name.to_string_lossy()));
    let _ = std::fs::remove_file(&retired);

    // A running executable can be renamed on Windows even though it cannot be overwritten: the
    // image stays mapped under the old name, so the path becomes free for the new copy.
    std::fs::rename(exe, &retired).map_err(|error| {
        let _ = std::fs::remove_dir_all(&staging);
        failure(format!("the running copy could not be moved aside: {error}"))
    })?;

    if let Err(error) = std::fs::copy(&staged_exe, exe) {
        // Put the running copy back under its own name before anything else notices.
        let _ = std::fs::rename(&retired, exe);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(failure(format!("the new copy could not be written: {error}")));
    }

    let staged_seed = staging.join(ARCHIVE_SEED);
    if staged_seed.is_file()
        && let Err(error) = std::fs::copy(&staged_seed, directory.join(ARCHIVE_SEED))
    {
        // The bridge is prepared again on the next start; refusing the whole update over it
        // would leave the old copy retired for nothing.
        log::warn!("the MCP bridge seed could not be replaced: {error}");
    }

    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// Unpacks the archive's top-level files. Entries that point anywhere but the folder itself are
/// refused rather than skipped: the archive is written by this project's release, so one that
/// tries to leave the folder is not this archive.
fn extract(archive: &[u8], into: &Path) -> Result<(), ShellError> {
    std::fs::create_dir_all(into)
        .map_err(|error| failure(format!("the update folder could not be created: {error}")))?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|error| failure(format!("the archive could not be read: {error}")))?;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| failure(format!("an archive entry could not be read: {error}")))?;
        let Some(name) = entry.enclosed_name() else {
            return Err(failure("an archive entry points outside the update folder"));
        };
        if entry.is_dir() || name.components().count() != 1 {
            continue;
        }
        let target = into.join(&name);
        let mut file = std::fs::File::create(&target)
            .map_err(|error| failure(format!("{} could not be written: {error}", target.display())))?;
        std::io::copy(&mut entry, &mut file)
            .map_err(|error| failure(format!("{} could not be unpacked: {error}", target.display())))?;
    }

    Ok(())
}

fn failure(detail: impl Into<String>) -> ShellError {
    ShellError::update_install_failed(detail.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip in the shape the release writes: every file at the top level, stored rather than
    /// compressed, because what is being tested is the unpacking rather than the codec.
    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write as _;

        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for (name, bytes) in entries {
                writer.start_file(*name, options).expect("the entry starts");
                writer.write_all(bytes).expect("the entry is written");
            }
            writer.finish().expect("the archive is finished");
        }
        buffer.into_inner()
    }

    #[test]
    fn the_manifest_names_the_portable_archive_for_this_platform() {
        let manifest = serde_json::json!({
            "platforms": {
                "windows-x86_64": { "url": "https://example.invalid/setup.exe", "signature": "installer" },
                "windows-x86_64-portable": { "url": "https://example.invalid/portable.zip", "signature": "archive" }
            }
        });

        let source = portable_entry(&manifest, "windows-x86_64").expect("the portable entry is named");

        assert_eq!(source.url.as_str(), "https://example.invalid/portable.zip");
        assert_eq!(source.signature, "archive");
        assert!(
            portable_entry(&manifest, "windows-aarch64").is_none(),
            "another platform's entry is not this copy's archive"
        );
    }

    #[test]
    fn a_manifest_without_the_portable_entry_is_not_a_portable_update() {
        let manifest = serde_json::json!({ "platforms": { "windows-x86_64": { "url": "https://example.invalid/a" } } });

        assert!(portable_entry(&manifest, "windows-x86_64").is_none());
    }

    #[test]
    fn an_entry_that_points_outside_the_update_folder_is_refused() {
        let into = tempfile::tempdir().expect("a folder");
        let escaping = archive(&[("../escaped.exe", b"x")]);

        let error = extract(&escaping, into.path()).expect_err("an entry that climbs out is not this archive");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::UpdateInstallFailed);
        assert!(
            !into.path().parent().expect("a parent").join("escaped.exe").exists(),
            "nothing may be written outside the folder the archive was given"
        );
    }

    #[test]
    fn the_running_copy_is_replaced_and_the_old_one_is_left_retired() {
        let folder = tempfile::tempdir().expect("a folder");
        let exe = folder.path().join("7Shell.exe");
        std::fs::write(&exe, b"old build").expect("the old copy");
        let update = archive(&[
            (ARCHIVE_EXE, b"new build"),
            (ARCHIVE_SEED, b"new seed"),
            ("README.md", b"notes"),
        ]);

        apply(&exe, &update).expect("the copy replaces itself");

        assert_eq!(std::fs::read(&exe).expect("the new copy"), b"new build");
        assert_eq!(
            std::fs::read(folder.path().join(format!("7Shell.exe{RETIRED_SUFFIX}"))).expect("the retired copy"),
            b"old build",
            "the copy that was running stays beside the new one until the next start removes it"
        );
        assert_eq!(
            std::fs::read(folder.path().join(ARCHIVE_SEED)).expect("the seed"),
            b"new seed"
        );
        assert!(
            !folder.path().join(STAGING_DIR).exists(),
            "the staging folder is not left behind"
        );
    }

    #[test]
    fn an_archive_without_the_executable_leaves_the_copy_alone() {
        let folder = tempfile::tempdir().expect("a folder");
        let exe = folder.path().join("7Shell.exe");
        std::fs::write(&exe, b"old build").expect("the old copy");
        let update = archive(&[("README.md", b"notes")]);

        let error = apply(&exe, &update).expect_err("an archive with no executable cannot replace anything");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::UpdateInstallFailed);
        assert_eq!(std::fs::read(&exe).expect("the untouched copy"), b"old build");
        assert!(!folder.path().join(STAGING_DIR).exists());
    }
}
