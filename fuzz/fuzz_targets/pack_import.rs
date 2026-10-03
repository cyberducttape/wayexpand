//! Untrusted snippet packs: the input is split at the first NUL into the
//! manifest and one snippet file. Import must fail cleanly or yield a valid
//! configuration with commands stripped.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::os::unix::fs::PermissionsExt;

fuzz_target!(|data: &[u8]| {
    let (manifest, snippets) = match data.iter().position(|&byte| byte == 0) {
        Some(split) => (&data[..split], &data[split + 1..]),
        None => (data, &[][..]),
    };
    let root = std::env::temp_dir().join(format!("wayexpand-fuzz-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let snippet_dir = root.join("snippets");
    if std::fs::create_dir_all(&snippet_dir).is_err() {
        return;
    }
    for directory in [&root, &snippet_dir] {
        let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
    }
    let manifest_path = root.join("wayexpand-pack.toml");
    let snippet_path = snippet_dir.join("main.toml");
    if std::fs::write(&manifest_path, manifest).is_err()
        || std::fs::write(&snippet_path, snippets).is_err()
    {
        return;
    }
    for file in [&manifest_path, &snippet_path] {
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
    }
    let _ = wayexpand_core::inspect_pack(&root);
    if let Ok((_, config, _)) = wayexpand_core::import_pack(&root) {
        assert!(
            config.expansion.iter().all(|expansion| expansion.command.is_none())
                && config.hotkey.is_empty(),
            "pack import must strip execution"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
});
