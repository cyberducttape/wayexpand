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

    assert!(examples > 0, "documentation must publish executable TOML examples");
}
