/// Build script for netshare-gui.
///
/// On Windows (MSVC toolchain) we embed a UAC manifest that requests
/// Administrator execution level.  This makes both the sharing machine AND
/// the receiving machine start with equal, elevated privileges so that
/// SendInput can inject mouse/keyboard events into *any* window — including
/// Task Manager, UAC dialogs, and other high-integrity processes — without
/// one side having a lower permission than the other.
///
/// On non-MSVC (GNU/MinGW, Linux, macOS) the manifest block is skipped;
/// Linux has no UIPI equivalent so no elevation is needed there.
fn main() {
    let target_os  = std::env::var("CARGO_CFG_TARGET_OS") .unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "windows" && target_env == "msvc" {
        // Write the manifest XML to the Cargo output directory, then tell
        // the MSVC linker to embed it directly into the PE binary.
        let out_dir       = std::env::var("OUT_DIR").expect("OUT_DIR not set");
        let manifest_path = std::path::PathBuf::from(&out_dir).join("netshare.manifest");

        std::fs::write(&manifest_path, MANIFEST_XML)
            .expect("failed to write UAC manifest");

        // /MANIFEST:EMBED  — embed rather than produce a sidecar .manifest file
        // /MANIFESTINPUT   — merge our trustInfo into the default manifest
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
    }

    println!("cargo:rerun-if-changed=build.rs");
}

/// UAC manifest fragment that tells Windows to require Administrator
/// privileges before launching the process.  Both machines in a NetShare
/// session embed the same manifest so they always run at equal integrity.
const MANIFEST_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;
