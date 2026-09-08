use super::MANIFEST;
use crate::support::Project;

#[test]
fn removal_boundary_warns_by_default_and_fails_when_denied() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1", remove = "2", replacement = "new")]
pub fn old() {}
pub fn new() {}
"#,
        ),
    ]);
    project
        .cli(&["check"])
        .success()
        .stderr("warning[deprecate::removal_due]");
    project
        .cli(&["check", "--deny", "overdue"])
        .failure()
        .stderr("error[deprecate::removal_due]")
        .stderr("1 deprecation policy violation(s)");
}

#[test]
fn prerelease_is_before_release_removal_boundary() {
    let project = Project::new(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "prerelease"
version = "2.0.0-beta.1"
edition = "2021"

[dependencies]
deprecate = { path = "$DEPRECATE" }
"#,
        ),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1", remove = "2", replacement = "new")]
pub fn old() {}
pub fn new() {}
"#,
        ),
    ]);
    project.cli(&["check", "--deny", "all"]).success();
}

#[test]
fn missing_replacement_policy_accepts_explicit_recipe() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1", migrate = "old() => new()")]
pub fn old() {}
pub fn new() {}
#[deprecate::item(since = "1")]
pub fn abandoned() {}
"#,
        ),
    ]);
    project
        .cli(&["check"])
        .success()
        .stderr("missing_replacement");
    project
        .cli(&["check", "--deny", "missing-replacement"])
        .failure()
        .stderr("1 deprecation policy violation(s)")
        .stderr("abandoned");
}

#[test]
fn repeated_deny_flags_and_all_enforce_both_policies() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1", remove = "2")]
pub fn old() {}
"#,
        ),
    ]);
    project
        .cli(&[
            "check",
            "--deny",
            "overdue",
            "--deny",
            "missing-replacement",
        ])
        .failure()
        .stderr("2 deprecation policy violation(s)");
    project
        .cli(&["check", "--deny", "all"])
        .failure()
        .stderr("2 deprecation policy violation(s)");
}

#[test]
fn unknown_features_and_replacements_are_errors() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
deprecate::features! {
    "missing" => {
        since: "1",
        replacement: "also-missing",
    },
}
"#,
        ),
    ]);
    project
        .cli(&["check"])
        .failure()
        .stderr("unknown_feature")
        .stderr("unknown_replacement")
        .stderr("2 deprecation policy violation(s)");
}

#[test]
fn enabled_deprecated_feature_is_reported_through_an_alias() {
    let project = Project::new(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "features"
version = "2.0.0"
edition = "2021"

[dependencies]
deprecate = { path = "$DEPRECATE" }

[features]
default = ["old"]
old = []
new = []
"#,
        ),
        (
            "src/lib.rs",
            r#"
use deprecate::features as lifecycle;
lifecycle! {
    "old" => {
        since: "1",
        replacement: "new",
    },
}
"#,
        ),
    ]);
    project.cargo(&["check"]).success();
    project
        .cli(&["check", "--deny", "all"])
        .success()
        .stderr("feature_enabled");
    project.cli(&["list"]).success().stdout("feature old");
}

#[test]
fn malformed_lifecycle_is_rejected_by_scanner_and_compiler() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "2", remove = "1")]
pub fn old() {}
"#,
        ),
    ]);
    project.cli(&["check"]).failure().stderr("must be later");
    project.cargo(&["check"]).failure().stderr("must be later");
}

#[test]
fn unknown_migration_placeholder_is_rejected_by_compiler() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1", migrate = "old($x) => new($other)")]
pub fn old(_: u32) {}
pub fn new(_: u32) {}
"#,
        ),
    ]);
    project
        .cargo(&["check"])
        .failure()
        .stderr("unknown migration placeholder");
    project.cli(&["fix"]).failure();
}
