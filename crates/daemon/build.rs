use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn git_output(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn git_commit(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!commit.is_empty()).then_some(commit)
}

fn main() {
    println!("cargo:rerun-if-env-changed=WAYEXPAND_BUILD_SHA");
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let mut watched = vec![
        "HEAD".to_owned(),
        "index".to_owned(),
        "packed-refs".to_owned(),
    ];
    watched.extend(git_output(&root, &["symbolic-ref", "-q", "HEAD"]));
    for name in watched {
        if let Some(path) = git_output(&root, &["rev-parse", "--git-path", &name])
            .map(PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                }
            })
            .filter(|path| path.exists())
        {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    let commit = env::var("WAYEXPAND_BUILD_SHA")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| git_commit(&root))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=WAYEXPAND_DAEMON_COMMIT={commit}");
}
