use std::os::unix::fs::PermissionsExt;
use wayexpand_core::Config;

#[test]
fn published_executable_toml_examples_parse_as_config() {
    let documentation = include_str!("../../../docs/CERTIFICATION_MATRIX.md");
    let marker = "<!-- executable-toml: config -->";
    let mut examples = 0;
    let mut remainder = documentation;

    while let Some(marker_offset) = remainder.find(marker) {
        remainder = &remainder[marker_offset + marker.len()..];
        let fence = remainder
            .find("```toml\n")
            .expect("executable TOML marker must precede a TOML fence");
        let source = &remainder[fence + "```toml\n".len()..];
        let end = source
            .find("\n```")
            .expect("executable TOML fence must be closed");
        Config::parse(&source[..end]).expect("published TOML example must load as Config");
        examples += 1;
        remainder = &source[end + "\n```".len()..];
    }

    assert!(
        examples > 0,
        "documentation must publish executable TOML examples"
    );
}

#[test]
fn ensure_user_config_creates_a_private_config_on_first_run() {
    let root = std::env::temp_dir().join(format!(
        "wayexpand-first-run-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos()
    ));
    let path = root.join("config/wayexpand/expansions.toml");
    std::fs::create_dir_all(root.join("config")).expect("test config home should exist");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .expect("test home should be private");
    std::fs::set_permissions(root.join("config"), std::fs::Permissions::from_mode(0o700))
        .expect("test config home should be private");

    let loaded = Config::ensure_user_config(&path).expect("first-run config should be created");
    assert!(loaded.config.expansion.is_empty());
    assert_eq!(
        std::fs::metadata(&path)
            .expect("first-run config should exist")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(Config::load(&path).is_ok());

    std::fs::remove_dir_all(root).expect("test config directory should be removable");
}
