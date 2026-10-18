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
    assert_eq!(
        insertion_from_status("running\ninsertion_assurance=compatibility\n"),
        Some(InsertionAssurance::Compatibility)
    );
    assert_eq!(
        insertion_from_status("running\ninsertion_assurance=blocked\n"),
        Some(InsertionAssurance::Blocked)
    );
    assert_eq!(
        insertion_from_status("running\ninsertion_assurance=unavailable\n"),
        None
    );
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

fn demo_app(snippets: usize) -> (App, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "wayexpand-tui-render-{snippets}-{}-{:?}.toml",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_file(&path);
    let mut text = String::from(
        "[[expansion]]\ntrigger = \";sig\"\nreplacement = \"Best regards,\\nAlex Morgan\\nPlatform Engineering\"\ndescription = \"Email signature\"\ntags = [\"email\", \"signature\"]\ncategory = \"Email\"\n\n\
         [[expansion]]\ntrigger = \";ty\"\nreplacement = \"Thanks for reaching out! \"\ndescription = \"Support reply opener with a description far too long to fit in any reasonable terminal column at all\"\nmatch_mode = \"word-boundary\"\n\n\
         [[expansion]]\ntrigger = \";off\"\nreplacement = \"disabled\\tvalue\"\nenabled = false\n",
    );
    for index in 0..snippets.saturating_sub(3) {
        text.push_str(&format!(
            "\n[[expansion]]\ntrigger = \";s{index}\"\nreplacement = \"value {index}\"\ndescription = \"Snippet {index}\"\n"
        ));
    }
    Config::parse(&text).unwrap().save_atomic(&path).unwrap();
    (App::load(path.clone()).unwrap(), path)
}

fn screen(app: &mut App, width: u16, height: u16) -> Vec<String> {
    render::plain_text(&frame_for(app, width, height))
}

fn assert_exact_size(lines: &[String], width: u16, height: u16) {
    use unicode_width::UnicodeWidthStr;
    assert_eq!(lines.len(), usize::from(height));
    for line in lines {
        assert_eq!(line.width(), usize::from(width), "{line:?}");
        assert!(!line.contains('\n') && !line.contains('\t'), "{line:?}");
    }
}

#[test]
fn every_frame_fills_the_terminal_exactly_at_any_size() {
    let (mut app, _path) = demo_app(40);
    for (width, height) in [(20, 5), (40, 8), (80, 24), (99, 30), (100, 30), (160, 50)] {
        for state in 0..4 {
            match state {
                1 => app.begin_edit(),
                2 => {
                    app.prompt = None;
                    app.show_help = true;
                }
                3 => {
                    app.show_help = false;
                    app.searching = true;
                    app.query = "no such snippet".into();
                }
                _ => {}
            }
            assert_exact_size(&screen(&mut app, width, height), width, height);
        }
        app.searching = false;
        app.query.clear();
    }
}

#[test]
fn list_scrolls_to_keep_the_selection_visible() {
    let (mut app, _path) = demo_app(100);
    update(&mut app, key(KeyCode::End)).unwrap();
    let lines = screen(&mut app, 80, 24);
    assert!(
        lines.iter().any(|line| line.contains("❯ ● ;s96")),
        "{lines:#?}"
    );
    update(&mut app, key(KeyCode::Home)).unwrap();
    let lines = screen(&mut app, 80, 24);
    assert!(
        lines.iter().any(|line| line.contains("❯ ● ;sig")),
        "{lines:#?}"
    );
    update(&mut app, key(KeyCode::PageDown)).unwrap();
    assert_eq!(app.selected, app.list_rows);
}

#[test]
fn details_show_multi_line_previews_on_their_own_rows() {
    let (mut app, _path) = demo_app(3);
    let lines = screen(&mut app, 80, 24);
    for expected in ["│ Best regards,", "│ Alex Morgan", "│ Platform Engineering"] {
        assert!(
            lines.iter().any(|line| line.contains(expected)),
            "{lines:#?}"
        );
    }
    // Side by side on a wide terminal.
    let lines = screen(&mut app, 120, 24);
    assert!(
        lines.iter().any(|line| line.contains("│ │ Best regards,")),
        "{lines:#?}"
    );
}

#[test]
fn long_text_is_cut_with_an_ellipsis_instead_of_wrapping() {
    let (mut app, _path) = demo_app(3);
    let lines = screen(&mut app, 80, 24);
    let row = lines.iter().find(|line| line.contains(";ty")).unwrap();
    assert!(row.contains('…') && row.contains("word"), "{row:?}");
}

#[test]
fn prompt_caret_edits_in_the_middle_and_stays_visible() {
    let (mut app, _path) = demo_app(3);
    update(&mut app, key(KeyCode::Char('D'))).unwrap();
    assert_eq!(app.input, "Email signature");
    for _ in 0.."signature".len() {
        update(&mut app, key(KeyCode::Left)).unwrap();
    }
    for character in "work ".chars() {
        update(&mut app, key(KeyCode::Char(character))).unwrap();
    }
    update(&mut app, key(KeyCode::Delete)).unwrap();
    assert_eq!(app.input, "Email work ignature");
    app.input = "x".repeat(300);
    app.cursor = 300;
    let lines = screen(&mut app, 60, 20);
    assert!(lines.last().unwrap().starts_with(" Description › …"));
}

#[test]
fn escape_clears_a_search_before_it_quits() {
    let (mut app, _path) = demo_app(3);
    for code in [
        KeyCode::Char('/'),
        KeyCode::Char('t'),
        KeyCode::Char('y'),
        KeyCode::Enter,
    ] {
        update(&mut app, key(code)).unwrap();
    }
    assert_eq!(app.visible_indices(), &[1]);
    assert_eq!(
        update(&mut app, key(KeyCode::Esc)).unwrap(),
        UiEffect::Continue
    );
    assert!(app.query.is_empty());
    assert_eq!(update(&mut app, key(KeyCode::Esc)).unwrap(), UiEffect::Quit);
}

#[test]
fn help_opens_with_question_mark_and_any_key_closes_it() {
    let (mut app, _path) = demo_app(3);
    update(&mut app, key(KeyCode::Char('?'))).unwrap();
    let lines = screen(&mut app, 80, 30);
    assert!(lines
        .iter()
        .any(|line| line.contains("pause or resume the daemon")));
    // The key that closes help does nothing else.
    update(&mut app, key(KeyCode::Char('d'))).unwrap();
    assert!(!app.show_help);
    assert_eq!(app.confirm_delete, None);
}

#[test]
fn home_directory_is_shown_as_a_tilde() {
    assert_eq!(
        display_path(
            std::path::Path::new("/home/user/.config/wayexpand/expansions.toml"),
            Some(std::path::Path::new("/home/user"))
        ),
        "~/.config/wayexpand/expansions.toml"
    );
    assert_eq!(
        display_path(
            std::path::Path::new("/etc/x.toml"),
            Some(std::path::Path::new(""))
        ),
        "/etc/x.toml"
    );
}

#[test]
#[ignore = "prints sample screens for a manual look"]
fn print_sample_screens() {
    let (mut app, _path) = demo_app(14);
    app.daemon = DaemonState::Running;
    app.insertion = Some(InsertionAssurance::Verified);
    for (width, height) in [(80, 24), (120, 30)] {
        println!("{}", screen(&mut app, width, height).join("\n"));
        println!("{}", "=".repeat(usize::from(width)));
    }
    app.show_help = true;
    println!("{}", screen(&mut app, 80, 30).join("\n"));
}
