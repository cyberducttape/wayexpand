use std::{env, path::Path, process::Command};

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

fn main() {
    println!("cargo:rerun-if-env-changed=WAYEXPAND_BUILD_SHA");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");

    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let package_version = env::var("CARGO_PKG_VERSION").unwrap();
    let sha = env::var("WAYEXPAND_BUILD_SHA")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| git_output(&root, &["rev-parse", "--short=12", "HEAD"]));
    let exact_release = sha.is_some()
        && git_output(
            &root,
            &[
                "describe",
                "--tags",
                "--exact-match",
                "--match",
                &format!("v{package_version}"),
            ],
        )
        .is_some()
        && Command::new("git")
            .args(["diff", "--quiet"])
            .current_dir(&root)
            .status()
            .is_ok_and(|status| status.success());

    let version = match sha.as_deref() {
        Some(_sha) if exact_release => package_version.clone(),
        Some(sha) => format!("{package_version}-dev+{sha}"),
        None => package_version.clone(),
    };
    let commit = sha.unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=WAYEXPAND_BUILD_VERSION={version}");
    println!("cargo:rustc-env=WAYEXPAND_COMMIT={commit}");
}
