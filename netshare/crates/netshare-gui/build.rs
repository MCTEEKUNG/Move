/// Build script for netshare-gui.
///
/// On Windows MSVC, we set the UAC execution level to `requireAdministrator`
/// so both machines in a NetShare session always start with equal, elevated
/// privileges.  This lets SendInput inject into any window (Task Manager,
/// UAC dialogs, etc.) without one peer having fewer rights than the other.
///
/// We use the linker's built-in /MANIFESTUAC flag instead of an external
/// manifest file — Rust already embeds a default manifest; this flag merges
/// our UAC level into it without conflict.
fn main() {
    let target_os  = std::env::var("CARGO_CFG_TARGET_OS") .unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "windows" && target_env == "msvc" {
        // /MANIFESTUAC:level=requireAdministrator
        // Tells the MSVC linker to embed a UAC requestedExecutionLevel of
        // requireAdministrator.  No external .manifest file needed; this
        // merges cleanly with the manifest Rust already produces.
        println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level=requireAdministrator");
    }

    println!("cargo:rerun-if-changed=build.rs");
}
