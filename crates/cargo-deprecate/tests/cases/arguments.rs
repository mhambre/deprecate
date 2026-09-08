use super::MANIFEST;
use crate::support::Project;

#[test]
fn help_and_version_work_outside_a_cargo_project() {
    let project = Project::new(&[]);
    project
        .cli(&["--help"])
        .success()
        .stdout("Usage:")
        .stdout("catalog");
    project
        .cli(&["deprecate", "--help"])
        .success()
        .stdout("Usage:");
    project
        .cli(&["--version"])
        .success()
        .stdout(env!("CARGO_PKG_VERSION"));
}

#[test]
fn malformed_arguments_fail_before_loading_a_project() {
    let project = Project::new(&[]);
    project
        .cli(&["not-a-command"])
        .failure()
        .stderr("unrecognized subcommand");
    project
        .cli(&["fix", "--unknown"])
        .failure()
        .stderr("unexpected argument");
    project.cli(&["diff"]).failure().stderr("--from");
    project
        .cli(&["check", "--deny", "unknown"])
        .failure()
        .stderr("invalid value");
}

#[test]
fn cargo_argv_and_direct_invocation_both_default_to_inventory() {
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", "")]);
    project
        .cli(&[])
        .success()
        .stdout("No structured deprecations");
    project
        .cli(&["deprecate"])
        .success()
        .stdout("No structured deprecations");
    project
        .cli(&["list"])
        .success()
        .stdout("No structured deprecations");
}
