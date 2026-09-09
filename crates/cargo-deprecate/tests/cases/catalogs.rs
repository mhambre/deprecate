use super::MANIFEST;
use crate::support::Project;
use std::path::Path;

/// A dependency cannot borrow recipes from another package's catalog.
#[test]
fn dependency_catalog_rejects_a_different_package_name() {
    assert_catalog_identity_rejected("unrelated", "2.0.0");
}

/// A stale catalog must not rewrite calls in a different dependency version.
#[test]
fn dependency_catalog_rejects_a_different_version() {
    assert_catalog_identity_rejected("dependency", "1.0.0");
}

/// Catalog versions use exact Cargo versions, not lifecycle shorthand.
#[test]
fn dependency_catalog_rejects_an_abbreviated_version() {
    assert_catalog_identity_rejected("dependency", "2");
}

/// Verifies every catalog-consuming command fails before changing source.
fn assert_catalog_identity_rejected(package: &str, version: &str) {
    let catalog = r#"
{
    "schema": 1,
    "crate": "$PACKAGE",
    "version": "$VERSION",
    "deprecations": [
        {
            "kind": "item",
            "name": "old",
            "since": "1",
            "replacement": "new"
        }
    ]
}
"#;
    let project = Project::new(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "consumer"
version = "1.0.0"
edition = "2021"

[dependencies]
renamed = { package = "dependency", path = "dependency" }
"#,
        ),
        (
            "src/lib.rs",
            r"
pub fn run() -> u32 {
    renamed::old()
}

#[test]
fn behavior() {
    assert_eq!(run(), 10);
}
",
        ),
        (
            "dependency/Cargo.toml",
            r#"
[package]
name = "dependency"
version = "2.0.0"
edition = "2021"
"#,
        ),
        (
            "dependency/src/lib.rs",
            r"
#[deprecated]
pub fn old() -> u32 {
    10
}

pub fn new() -> u32 {
    20
}
",
        ),
        (
            "dependency/deprecations.json",
            &catalog
                .replace("$PACKAGE", package)
                .replace("$VERSION", version),
        ),
    ]);
    project.cargo(&["test"]).success();
    for args in [
        vec!["list", "--dependencies"],
        vec!["check", "--dependencies", "--deny", "overdue"],
        vec!["fix", "--dry-run"],
        vec!["fix"],
    ] {
        project
            .cli(&args)
            .failure()
            .stderr("catalog identity mismatch")
            .stderr("expected dependency@2.0.0")
            .stderr(&format!("found {package}@{version}"));
        project.assert_unchanged();
    }
    project.cargo(&["test"]).success();
}

#[test]
fn catalog_round_trip_is_stable_and_preserves_all_fields() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(
    since = "1",
    remove = "3",
    replacement = "new",
    reason = "use the typed result",
    migrate = "old($x) => new($x)"
)]
pub fn old(x: u32) -> u32 {
    x
}
pub fn new(x: u32) -> u32 {
    x
}
"#,
        ),
    ]);
    project.cli(&["catalog"]).success();
    let first = project.read("deprecations.json");
    let catalog = project.json("deprecations.json");
    assert_eq!(
        catalog,
        serde_json::json!({
            "schema": 1, "crate": "fixture", "version": "2.0.0",
            "deprecations": [{
                "kind": "item", "name": "old", "since": "1", "remove": "3",
                "replacement": "new", "reason": "use the typed result",
                "migrate": "old($x) => new($x)"
            }]
        })
    );
    project.cli(&["catalog"]).success();
    project.assert_file("deprecations.json", &first);
    project
        .cli(&["diff", "--from", "deprecations.json"])
        .success()
        .stdout("0 changes");
}

#[test]
fn diff_reports_added_removed_and_changed_declarations() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1", remove = "3", replacement = "new")]
pub fn changed() {}
#[deprecate::item(since = "1")]
pub fn removed() {}
"#,
        ),
    ]);
    project
        .cli(&["catalog", "--output", "baseline.json"])
        .success();
    project.write(
        "src/lib.rs",
        r#"
#[deprecate::item(since = "1", remove = "4", replacement = "better")]
pub fn changed() {}
#[deprecate::item(since = "2")]
pub fn added() {}
"#,
    );
    project
        .cli(&["diff", "--from", "baseline.json"])
        .success()
        .stdout("+ added")
        .stdout("- removed")
        .stdout("~ changed")
        .stdout("removal 3 -> 4")
        .stdout("replacement new -> better")
        .stdout("3 changes");
}

#[test]
fn workspace_diff_compares_only_the_baseline_package() {
    let project = Project::new(&[
        (
            "Cargo.toml",
            r#"
[workspace]
members = ["a", "b"]
resolver = "2"
"#,
        ),
        (
            "a/Cargo.toml",
            r#"
[package]
name = "a"
version = "1.0.0"
edition = "2021"

[dependencies]
deprecate = { path = "$DEPRECATE" }
"#,
        ),
        (
            "a/src/lib.rs",
            r#"
#[deprecate::item(since = "1")]
pub fn old() {}
"#,
        ),
        (
            "b/Cargo.toml",
            r#"
[package]
name = "b"
version = "1.0.0"
edition = "2021"

[dependencies]
deprecate = { path = "$DEPRECATE" }
"#,
        ),
        (
            "b/src/lib.rs",
            r#"
#[deprecate::item(since = "1", replacement = "different")]
pub fn old() {}
#[deprecate::item(since = "1")]
pub fn only_b() {}
"#,
        ),
    ]);
    let a_catalog = Path::new("a/deprecations.json").display().to_string();
    let b_catalog = Path::new("b/deprecations.json").display().to_string();
    project
        .cli(&["catalog"])
        .success()
        .stdout(&a_catalog)
        .stdout(&b_catalog);
    project
        .cli(&["diff", "--from", "a/deprecations.json"])
        .success()
        .stdout("0 changes")
        .stdout_excludes("only_b");
    project
        .cli(&["catalog", "--output", "combined.json"])
        .failure()
        .stderr("--output requires");
}

#[test]
fn unknown_schema_and_invalid_lifecycle_are_rejected() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", ""),
        (
            "unknown.json",
            r#"
{
    "schema": 999,
    "crate": "fixture",
    "version": "2.0.0",
    "deprecations": []
}
"#,
        ),
        (
            "invalid.json",
            r#"
{
    "schema": 1,
    "crate": "fixture",
    "version": "2.0.0",
    "deprecations": [
        {
            "kind": "item",
            "name": "old",
            "since": "2",
            "remove": "1"
        }
    ]
}
"#,
        ),
        (
            "malformed.json",
            r#"
{"schema":
"#,
        ),
    ]);
    project
        .cli(&["diff", "--from", "unknown.json"])
        .failure()
        .stderr("unsupported catalog schema 999");
    project
        .cli(&["diff", "--from", "invalid.json"])
        .failure()
        .stderr("must be later");
    project
        .cli(&["diff", "--from", "malformed.json"])
        .failure()
        .stderr("failed to parse");
    project
        .cli(&["diff", "--from", "missing.json"])
        .failure()
        .stderr("failed to read");
}

#[test]
fn unknown_fields_are_compatible_but_unknown_packages_are_not() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1")]
pub fn old() {}
"#,
        ),
        (
            "extended.json",
            r#"
{
    "schema": 1,
    "crate": "fixture",
    "version": "2.0.0",
    "future": true,
    "deprecations": [
        {
            "kind": "item",
            "name": "old",
            "since": "1",
            "extra": 42
        }
    ]
}
"#,
        ),
        (
            "foreign.json",
            r#"
{
    "schema": 1,
    "crate": "not-in-workspace",
    "version": "1.0.0",
    "deprecations": []
}
"#,
        ),
    ]);
    project
        .cli(&["diff", "--from", "extended.json"])
        .success()
        .stdout("0 changes");
    project
        .cli(&["diff", "--from", "foreign.json"])
        .failure()
        .stderr("not in the current workspace");
}

#[test]
fn dependency_catalog_takes_precedence_over_source() {
    let project = Project::new(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "consumer"
version = "1.0.0"
edition = "2021"

[dependencies]
dependency = { path = "dependency" }
"#,
        ),
        ("src/lib.rs", ""),
        (
            "dependency/Cargo.toml",
            r#"
[package]
name = "dependency"
version = "2.0.0"
edition = "2021"

[dependencies]
deprecate = { path = "$DEPRECATE" }
"#,
        ),
        (
            "dependency/src/lib.rs",
            r#"
#[deprecate::item(since = "1")]
pub fn source_only() {}
"#,
        ),
        (
            "dependency/deprecations.json",
            r#"
{
    "schema": 1,
    "crate": "dependency",
    "version": "2.0.0",
    "deprecations": [
        {
            "kind": "item",
            "name": "catalog_only",
            "since": "1",
            "remove": "2"
        }
    ]
}
"#,
        ),
    ]);
    project
        .cli(&["list", "--dependencies"])
        .success()
        .stdout("catalog_only")
        .stdout_excludes("source_only");
    project
        .cli(&["check", "--dependencies", "--deny", "overdue"])
        .failure()
        .stderr("catalog_only");
}

#[test]
fn workspace_catalog_is_refreshed_from_source() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1")]
pub fn current() {}
"#,
        ),
        (
            "deprecations.json",
            r#"
{
    "schema": 1,
    "crate": "fixture",
    "version": "1.0.0",
    "deprecations": [
        {
            "kind": "item",
            "name": "stale",
            "since": "1"
        }
    ]
}
"#,
        ),
    ]);
    project.cli(&["catalog"]).success();
    assert_eq!(
        project.json("deprecations.json")["deprecations"][0]["name"],
        "current"
    );
    project
        .cli(&["diff", "--from", "deprecations.json"])
        .success()
        .stdout("0 changes");
}
