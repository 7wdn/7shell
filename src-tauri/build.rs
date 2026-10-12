fn main() {
    // The icon is part of the binary, not a file next to it: tauri-build writes the .ico into the
    // Windows executable as a resource, and both the window and the tray take what they show from it
    // (`app.default_window_icon`). What tauri-build does not do is ask cargo to run this script again
    // when that file changes — it declares tauri.conf.json and capabilities and nothing else — so a
    // regenerated icon would sit in the folder while every binary kept the artwork it was linked
    // with. That failure has no error message: it reads as "the new icon did not show up".
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build();
}
