//! Untrusted Espanso match files: import must fail cleanly, and anything it
//! accepts must be a valid configuration.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::os::unix::fs::PermissionsExt;

fuzz_target!(|data: &[u8]| {
    let path = std::env::temp_dir().join(format!("wayexpand-fuzz-espanso-{}.yml", std::process::id()));
    if std::fs::write(&path, data).is_err() {
        return;
    }
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    if let Ok(import) = wayexpand_core::import_espanso(&path) {
        import
            .config
            .validate()
            .expect("an imported configuration is valid");
    }
    let _ = std::fs::remove_file(&path);
});
