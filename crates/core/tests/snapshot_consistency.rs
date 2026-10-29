//! A library must expand identically whichever path builds its validated
//! snapshot: the fleet merge the daemon loads, and the same merged
//! configuration saved and loaded again, as a reload does.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wayexpand_core::{Config, ExpansionEngine, FleetConfig, InputEvent, Layer};

fn private_directory(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn write_private(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn expand_all(engine: &mut ExpansionEngine, triggers: &[&str]) -> Vec<(String, Option<usize>)> {
    triggers
        .iter()
        .map(|trigger| {
            let results = engine.process(InputEvent::Text((*trigger).into()));
            assert_eq!(results.len(), 1, "{trigger} must expand");
            (results[0].insert.clone(), results[0].cursor_offset)
        })
        .collect()
}

#[test]
fn fleet_merge_and_saved_reload_produce_the_same_expansions() {
    let root: PathBuf = std::env::temp_dir().join(format!(
        "wayexpand-snapshot-consistency-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let user = root.join("user");
    let pack = root.join("pack");
    private_directory(&root);
    private_directory(&user);
    private_directory(&pack);
    write_private(
        &user.join("personal.toml"),
        r#"
[[expansion]]
trigger = ";sig"
replacement = "-- {{username}}"

[[expansion]]
trigger = ";wrap"
replacement = "<b>{{cursor}}</b> {{snippet:;sig}}"

[[expansion]]
trigger = ";braces"
replacement = "{{{{ .Values.name }}}}{{tab}}x"
"#,
    );
    write_private(
        &pack.join("team.toml"),
        r#"
[[expansion]]
trigger = ";addr"
replacement = "1 Example Way{{newline}}Springfield"
"#,
    );

    let fleet = FleetConfig::load_layers(vec![
        (Layer::User, user.clone()),
        (Layer::Pack, pack.clone()),
    ])
    .unwrap();
    let merged = fleet.config;
    let fleet_snapshot = Arc::new(merged.validate_snapshot().unwrap());

    let saved = root.join("expansions.toml");
    merged.save_atomic(&saved).unwrap();
    let reloaded = Config::load(&saved).unwrap();
    let reload_snapshot = Arc::new(reloaded.validate_snapshot().unwrap());

    assert_eq!(
        fleet_snapshot.normalized_ids(),
        reload_snapshot.normalized_ids()
    );
    let triggers = [";sig", ";wrap", ";braces", ";addr"];
    let mut from_fleet = ExpansionEngine::from_validated(Arc::clone(&fleet_snapshot));
    let mut from_reload = ExpansionEngine::from_validated(reload_snapshot);
    let mut from_config = ExpansionEngine::new(merged).unwrap();
    let expected = expand_all(&mut from_fleet, &triggers);
    assert_eq!(expand_all(&mut from_reload, &triggers), expected);
    assert_eq!(expand_all(&mut from_config, &triggers), expected);

    let username = &expected[0].0["-- ".len()..];
    assert_eq!(
        expected[1],
        (
            format!("<b></b> -- {username}"),
            Some("</b> -- ".len() + username.chars().count())
        )
    );
    assert_eq!(expected[2].0, "{{ .Values.name }}\tx");
    assert_eq!(expected[3].0, "1 Example Way\nSpringfield");
    std::fs::remove_dir_all(root).unwrap();
}
