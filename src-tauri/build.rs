use embed_manifest::manifest::ExecutionLevel;
use embed_manifest::{embed_manifest, new_manifest};
use tauri_build::{try_build, Attributes, WindowsAttributes};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let level = if std::env::var("PROFILE").as_deref() == Ok("release") {
            ExecutionLevel::RequireAdministrator
        } else {
            ExecutionLevel::AsInvoker
        };
        let manifest = new_manifest("SysPulse").requested_execution_level(level);
        if let Err(e) = embed_manifest(manifest) {
            println!("cargo:warning=embed_manifest failed: {e}");
        }
    }

    let attrs = Attributes::new().windows_attributes(WindowsAttributes::new_without_app_manifest());
    if let Err(e) = try_build(attrs) {
        panic!("tauri_build failed: {e}");
    }
}
