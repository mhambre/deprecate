use super::MANIFEST;
use crate::support::Project;

#[test]
fn required_since_is_enforced_by_macro_and_scanner() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(replacement = "new")]
pub fn old() {}
"#,
        ),
    ]);
    project
        .cargo(&["check"])
        .failure()
        .stderr("missing required");
    project.cli(&["check"]).failure().stderr("missing required");
}

#[test]
fn duplicate_fields_are_rejected_by_macro_and_scanner() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1", since = "2")]
pub fn old() {}
"#,
        ),
    ]);
    project
        .cargo(&["check"])
        .failure()
        .stderr("duplicate deprecation field");
    project
        .cli(&["check"])
        .failure()
        .stderr("duplicate deprecation field");
}

#[test]
fn unknown_fields_are_rejected_by_macro_and_scanner() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1", typo = "new")]
pub fn old() {}
"#,
        ),
    ]);
    project
        .cargo(&["check"])
        .failure()
        .stderr("unknown deprecation field");
    project
        .cli(&["check"])
        .failure()
        .stderr("unknown deprecation field");
}

#[test]
fn invalid_semver_is_rejected_by_macro_and_scanner() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "01.2.3")]
pub fn old() {}
"#,
        ),
    ]);
    project
        .cargo(&["check"])
        .failure()
        .stderr("semantic version");
    project.cli(&["check"]).failure().stderr("invalid");
}

#[test]
fn duplicate_recipe_parameters_are_rejected() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(since = "1", migrate = "old($x, $x) => new($x)")]
pub fn old(_: u32, _: u32) {}
pub fn new(_: u32) {}
"#,
        ),
    ]);
    project
        .cargo(&["check"])
        .failure()
        .stderr("parameters must be unique");
}

#[test]
fn macro_preserves_items_and_emits_actionable_compiler_warnings() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
#[deprecate::item(
    since = "1",
    remove = "3",
    replacement = "new",
    reason = "the result is now typed"
)]
pub fn old(x: u32) -> u32 {
    x + 1
}
pub fn new(x: u32) -> u32 {
    x + 1
}
pub fn run() -> u32 {
    old(41)
}
#[test]
fn behavior() {
    assert_eq!(run(), 42);
}
"#,
        ),
    ]);
    project
        .cargo(&["test"])
        .success()
        .stderr("the result is now typed")
        .stderr("use `new` instead")
        .stderr("scheduled for removal in 3");
    project.assert_unchanged();
}
