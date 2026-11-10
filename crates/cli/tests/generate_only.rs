use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wayexpand-generate-only-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create private test directory");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("restrict test directory permissions");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_cli(directory: &Path, config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wayexpand"))
        .arg("new")
        .args(args)
        .arg("--config")
        .arg(config)
        .current_dir(directory)
        .env("HOME", directory)
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .output()
        .expect("run wayexpand CLI")
}

#[test]
fn generate_only_script_prints_cleanly_and_writes_without_overwriting() {
    let directory = TestDirectory::new();
    let config = directory.path().join("expansions.toml");
    fs::write(
        &config,
        r##"[[expansion]]
id = "a6211c93-198e-4726-9785-4a82e419c465"
trigger = ";deploy-script"
replacement = "#!/bin/sh\nprintf '%s\\n' '{{field:service=api}}'\n"
delivery = "generate_only"
output_file = "deploy.sh"
"##,
    )
    .expect("write test configuration");
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
        .expect("restrict test configuration permissions");
    let expected = b"#!/bin/sh\nprintf '%s\\n' 'worker'\n";

    let printed = run_cli(
        directory.path(),
        &config,
        &["deploy-script", "--stdout", "--set", "service=worker"],
    );
    assert!(
        printed.status.success(),
        "{}",
        String::from_utf8_lossy(&printed.stderr)
    );
    assert_eq!(printed.stdout, expected);
    assert!(printed.stderr.is_empty());

    let written = run_cli(
        directory.path(),
        &config,
        &["deploy-script", "--set", "service=worker"],
    );
    assert!(
        written.status.success(),
        "{}",
        String::from_utf8_lossy(&written.stderr)
    );
    assert!(
        written.stdout.is_empty(),
        "file generation must not emit script content on stdout"
    );
    assert_eq!(
        fs::read(directory.path().join("deploy.sh")).unwrap(),
        expected
    );
    assert_eq!(
        fs::metadata(directory.path().join("deploy.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let rejected = run_cli(
        directory.path(),
        &config,
        &["deploy-script", "--set", "service=changed"],
    );
    assert!(
        !rejected.status.success(),
        "existing files must not be overwritten implicitly"
    );
    assert_eq!(
        fs::read(directory.path().join("deploy.sh")).unwrap(),
        expected
    );
}
