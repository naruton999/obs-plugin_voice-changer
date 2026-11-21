fn main() {
    // The plugin is loaded by OBS, which already has libobs in its process.
    // Resolve obs_* symbols at load time instead of linking a libobs binary.
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-arg=-Wl,-undefined,dynamic_lookup");
    }
}
