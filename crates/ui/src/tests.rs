use super::*;

fn test_app() -> (App, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-tui-reducer-{}-{:?}.toml",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_file(&path);
    let config = Config::parse(
            "[[expansion]]\ntrigger = \":one\"\nreplacement = \"first\"\n\n[[expansion]]\ntrigger = \":two\"\nreplacement = \"second\"\n",
        )
        .unwrap();
    config.save_atomic(&path).unwrap();
    (App::load(path.clone()).unwrap(), path)
}

fn test_app_with_triggers(triggers: &[&str]) -> (App, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-tui-reducer-many-{}-{:?}.toml",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_file(&path);
    let config = Config::parse(
        &triggers
            .iter()
            .enumerate()
            .map(|(index, trigger)| {
                format!("[[expansion]]\ntrigger = \"{trigger}\"\nreplacement = \"value-{index}\"\n")
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    config.save_atomic(&path).unwrap();
    (App::load(path.clone()).unwrap(), path)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn reducer_delete_cancel_preserves_selection_and_config() {
    let (mut app, path) = test_app();
    update(&mut app, key(KeyCode::Char('d'))).unwrap();
    assert_eq!(app.confirm_delete, Some(0));
    update(&mut app, key(KeyCode::Esc)).unwrap();
    assert_eq!(app.confirm_delete, None);
    assert_eq!(app.config.expansion.len(), 2);
    assert_eq!(Config::load(path).unwrap().expansion.len(), 2);
}

#[test]
fn reducer_delete_confirm_then_undo_restores_the_config() {
    let (mut app, path) = test_app();
    update(&mut app, key(KeyCode::Char('d'))).unwrap();
    update(&mut app, key(KeyCode::Char('d'))).unwrap();
    assert_eq!(app.config.expansion.len(), 1);
    update(&mut app, key(KeyCode::Char('u'))).unwrap();
    assert_eq!(app.config.expansion.len(), 2);
    assert_eq!(Config::load(path).unwrap().expansion.len(), 2);
}

#[test]
fn reducer_delete_last_visible_item_clamps_selection_after_cache_invalidation() {
    let (mut app, _path) = test_app();
    app.selected = 1;
    assert_eq!(app.visible_indices(), &[0, 1]);

    app.delete_confirmed(1);

    assert_eq!(app.config.expansion.len(), 1);
    assert_eq!(app.selected, 0);
    assert_eq!(app.visible_indices(), &[0]);
}

#[test]
fn reducer_delete_while_filtering_invalidates_visible_cache() {
    let (mut app, _path) = test_app();
    app.query = "two".to_string();
    assert_eq!(app.visible_indices(), &[1]);

    app.delete_confirmed(1);

    assert!(app.visible_indices().is_empty());
    assert_eq!(app.selected, 0);
}

#[test]
fn reducer_undo_while_filtering_rebuilds_visible_cache() {
    let (mut app, _path) = test_app();
    app.query = "two".to_string();
    assert_eq!(app.visible_indices(), &[1]);
    app.delete_confirmed(1);
    assert!(app.visible_indices().is_empty());

    app.undo_last();

    assert_eq!(app.visible_indices(), &[1]);
    assert_eq!(app.selected, 0);
}

#[test]
fn reducer_delete_before_selection_preserves_following_item() {
    let (mut app, _path) = test_app_with_triggers(&[":one", ":two", ":three"]);
    app.selected = 2;
    assert_eq!(app.selected_index(), Some(2));

    app.delete_confirmed(0);

    assert_eq!(app.selected, 1);
    assert_eq!(app.selected_index(), Some(1));
    assert_eq!(app.config.expansion[1].trigger, ":three");
}

#[test]
fn reducer_search_then_edit_updates_the_matching_snippet() {
    let (mut app, path) = test_app();
    update(&mut app, key(KeyCode::Char('/'))).unwrap();
    update(&mut app, key(KeyCode::Char('t'))).unwrap();
    update(&mut app, key(KeyCode::Char('w'))).unwrap();
    update(&mut app, key(KeyCode::Enter)).unwrap();
    update(&mut app, key(KeyCode::Char('e'))).unwrap();
    update(
        &mut app,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
    )
    .unwrap();
    update(&mut app, key(KeyCode::Char('u'))).unwrap();
    update(&mut app, key(KeyCode::Char('p'))).unwrap();
    update(&mut app, key(KeyCode::Char('d'))).unwrap();
    update(&mut app, key(KeyCode::Char('a'))).unwrap();
    update(&mut app, key(KeyCode::Char('t'))).unwrap();
    update(&mut app, key(KeyCode::Char('e'))).unwrap();
    update(&mut app, key(KeyCode::Enter)).unwrap();
    assert_eq!(app.config.expansion[1].replacement, "update");
    assert_eq!(
        Config::load(path).unwrap().expansion[1].replacement,
        "update"
    );
}

#[test]
fn external_edits_prefer_the_private_runtime_directory() {
    let runtime = std::env::temp_dir();
    assert_eq!(
        external_edit_directory(Some(runtime.clone().into_os_string())),
        runtime
    );
    // Unset, relative, or missing runtime directories fall back to /tmp.
    assert_eq!(external_edit_directory(None), std::env::temp_dir());
    assert_eq!(
        external_edit_directory(Some("relative/dir".into())),
        std::env::temp_dir()
    );
    assert_eq!(
        external_edit_directory(Some("/nonexistent/wayexpand-runtime".into())),
        std::env::temp_dir()
    );
}

#[test]
fn tui_status_uses_the_shared_daemon_status_contract() {
    assert_eq!(paused_from_status("running\npaused=true\n"), Some(true));
    assert_eq!(paused_from_status("running\npaused=false\n"), Some(false));
    assert_eq!(paused_from_status("running\npaused=maybe\n"), None);
    assert_eq!(paused_from_status("running\nwarning: paused=true\n"), None);
}

use std::fs;

#[test]
fn tui_tag_editor_round_trips_exact_values() {
    let tags = vec![
        "customer, west".into(),
        " email ".into(),
        String::new(),
        "line one\nline two".into(),
    ];
    assert_eq!(decode_tags(&encode_tags(&tags)).unwrap(), tags);
}

#[test]
fn tui_tag_editor_accepts_plain_comma_separated_tags() {
    assert_eq!(decode_tags(" edited,  ui ,, ").unwrap(), ["edited", "ui"]);
    assert_eq!(decode_tags("").unwrap(), Vec::<String>::new());
    // Simple tags are offered back in the same plain form...
    let simple = vec!["ops".to_owned(), "email".to_owned()];
    assert_eq!(encode_tags(&simple), "ops, email");
    assert_eq!(decode_tags(&encode_tags(&simple)).unwrap(), simple);
    // ...and a tag the plain form cannot carry switches to JSON.
    let comma = vec!["customer, west".to_owned()];
    assert_eq!(encode_tags(&comma), r#"["customer, west"]"#);
    assert!(decode_tags("[not json").is_err());
}

#[test]
fn tui_refuses_to_overwrite_a_newer_external_revision() {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-tui-revision-{}.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let initial =
        Config::parse("[[expansion]]\ntrigger = \":x\"\nreplacement = \"initial\"\n").unwrap();
    initial.save_atomic(&path).unwrap();
    let mut app = App::load(path.clone()).unwrap();

    let mut external = initial;
    external.expansion[0].replacement = "external edit".into();
    external.save_atomic(&path).unwrap();

    app.toggle_selected();
    let on_disk = Config::load(&path).unwrap();
    assert_eq!(on_disk.expansion[0].replacement, "external edit");
    assert!(on_disk.expansion[0].enabled);
    assert!(app.config.expansion[0].enabled);
    assert!(app.message.contains("changed externally"));

    let filename = path.file_name().unwrap().to_string_lossy();
    let lock_path = path.with_file_name(format!(".{filename}.wayexpand.lock"));
    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}
