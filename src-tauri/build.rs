fn main() {
    // The icon is part of the binary, not a file next to it: tauri-build writes the .ico into the
    // Windows executable as a resource, and both the window and the tray take what they show from it
    // (`app.default_window_icon`). What tauri-build does not do is ask cargo to run this script again
    // when that file changes — it declares tauri.conf.json and capabilities and nothing else — so a
    // regenerated icon would sit in the folder while every binary kept the artwork it was linked
    // with. That failure has no error message: it reads as "the new icon did not show up".
    println!("cargo:rerun-if-changed=icons");
    ensure_mcp_sidecar();
    tauri_build::build();
}

/// Gives `tauri-build` the resource it checks for, without pretending the bridge is installed.
///
/// `tauri.conf.json` declares the MCP bridge seed as an `externalBin`, and `tauri-build` refuses to
/// compile when that file is absent. The real bridge is produced by `pnpm build:sidecar` before a
/// bundle and by `beforeDevCommand` before a dev run; a plain `cargo check` or `cargo test` runs
/// neither, and a fresh checkout has no built binary to copy. An empty file satisfies the
/// compile-time check and is never what the application serves: `bridge_path` refuses a zero-byte
/// seed, and both Tauri commands replace this file with the real one before the application is
/// built.
fn ensure_mcp_sidecar() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.is_empty() {
        return;
    }

    let extension = if target.contains("windows") { ".exe" } else { "" };
    let path = std::path::Path::new("binaries").join(format!("sshell-mcp-seed-{target}{extension}"));
    if path.exists() {
        return;
    }

    if let Err(error) = std::fs::create_dir_all("binaries") {
        println!("cargo:warning=the MCP sidecar directory could not be created: {error}");
        return;
    }
    if let Err(error) = std::fs::write(&path, []) {
        println!("cargo:warning=the MCP sidecar placeholder could not be written: {error}");
    }
}
