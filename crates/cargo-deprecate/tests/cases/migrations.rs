use super::MANIFEST;
use crate::support::Project;

#[test]
fn module_alias_is_preserved_in_the_replacement() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
}
use api as legacy;
pub fn run() {
    legacy::old();
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.assert_file(
        "src/lib.rs",
        &source.replace("legacy::old()", "legacy::new()"),
    );
    project.cargo(&["check"]).success();
}

#[test]
fn dependency_catalog_supplies_recipe_without_annotation_dependency() {
    let source = r"
pub fn run() {
    dependency::old();
}
";
    let dependency = r#"
#[deprecated(note = "use new")]
pub fn old() {}
pub fn new() {}
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
dependency = { path = "dependency" }
"#,
        ),
        ("src/lib.rs", source),
        (
            "dependency/Cargo.toml",
            r#"
[package]
name = "dependency"
version = "1.0.0"
edition = "2021"
"#,
        ),
        ("dependency/src/lib.rs", dependency),
        (
            "dependency/deprecations.json",
            r#"
{
    "schema": 1,
    "crate": "dependency",
    "version": "1.0.0",
    "deprecations": [
        {
            "kind": "item",
            "name": "old",
            "since": "1",
            "migrate": "old() => new()"
        }
    ]
}
"#,
        ),
    ]);
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.assert_file(
        "src/lib.rs",
        &source.replace("dependency::old()", "dependency::new()"),
    );
    project.assert_file("dependency/src/lib.rs", dependency);
    project.cargo(&["check"]).success();
}

#[test]
fn direct_replacement_preserves_module_and_dry_run_is_read_only() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old(x: u32) -> u32 {
        x
    }
    pub fn new(x: u32) -> u32 {
        x
    }
}
pub fn consumer() -> u32 {
    api::old(41)
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["list"])
        .success()
        .stdout("fixture: api::old")
        .stdout("usages      1");
    project
        .cli(&["fix", "--dry-run"])
        .success()
        .stdout("api::new(41)");
    project.assert_unchanged();
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.assert_file(
        "src/lib.rs",
        &source.replace("api::old(41)", "api::new(41)"),
    );
    project.cargo(&["test"]).success();
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
}

#[test]
fn explicit_recipe_preserves_precedence_placeholders_and_literals() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", migrate = "old($x, $xy) => new($x * 2, $xy, \"$x\")")]
    pub fn old(x: u32, xy: u32) -> u32 {
        x * 2 + xy
    }
    pub fn new(x: u32, xy: u32, label: &str) -> u32 {
        assert_eq!(label, "$x");
        x + xy
    }
}
pub fn consumer() -> u32 {
    api::old(1 + 2, 4)
}
#[test]
fn behavior() {
    assert_eq!(consumer(), 10);
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.assert_file(
        "src/lib.rs",
        &source.replace("api::old(1 + 2, 4)", "api::new((1 + 2) * 2, 4, \"$x\")"),
    );
    project.cargo(&["test"]).success();
}

#[test]
fn edits_multiple_files_without_touching_comments_or_strings() {
    let api = r#"
#[deprecate::item(since = "1", replacement = "new")]
pub fn old(x: u32) -> u32 {
    x
}
pub fn new(x: u32) -> u32 {
    x
}
"#;
    let root = r"
pub mod api;
pub mod consumer;
pub fn run() -> u32 {
    api::old(1) + api::old(2)
}
";
    let consumer = r#"
use crate::api;
// api::old(99)
pub const TEXT: &str = "api::old(99)";
pub fn run() -> u32 {
    api::old(3)
}
"#;
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", root),
        ("src/api.rs", api),
        ("src/consumer.rs", consumer),
    ]);
    project.cli(&["fix"]).success().stdout("3 usages migrated");
    project.assert_file("src/lib.rs", &root.replace("api::old(", "api::new("));
    project.assert_file(
        "src/consumer.rs",
        &consumer.replace("api::old(3)", "api::new(3)"),
    );
    project.assert_file("src/api.rs", api);
    project.cargo(&["check"]).success();
}

#[test]
fn imported_calls_and_aliases_remain_manual() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
}
use api::{old, old as legacy};
pub fn run() {
    old();
    legacy();
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("4 usages require manual migration");
    project.assert_file("src/lib.rs", source);
    project.cargo(&["check"]).success();
}

#[test]
fn unrelated_deprecated_function_does_not_receive_a_recipe() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
}
pub mod unrelated {
    pub mod api {
        #[deprecated]
        pub fn old() {}
    }
}
use unrelated::api::old;
pub fn run() {
    old();
    unrelated::api::old();
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_file("src/lib.rs", source);
}

#[test]
fn dependency_recipe_is_scoped_to_its_owner() {
    let source = r"
pub mod api {
    #[deprecated]
    pub fn old() {}
}
pub fn run() {
    api::old();
    dependency::api::old();
}
";
    let dependency = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
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
dependency = { path = "dependency" }
"#,
        ),
        ("src/lib.rs", source),
        (
            "dependency/Cargo.toml",
            r#"
[package]
name = "dependency"
version = "1.0.0"
edition = "2021"

[dependencies]
deprecate = { path = "$DEPRECATE" }
"#,
        ),
        ("dependency/src/lib.rs", dependency),
    ]);
    project
        .cli(&["fix", "--workspace-only"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_file("src/lib.rs", source);
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.assert_file(
        "src/lib.rs",
        &source.replace("dependency::api::old()", "dependency::api::new()"),
    );
    project.assert_file("dependency/src/lib.rs", dependency);
    project.cargo(&["check"]).success();
}

#[test]
fn generic_calls_are_left_manual() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old<T: Default>() -> T {
        T::default()
    }
    pub fn new<T: Default>() -> T {
        T::default()
    }
}
pub fn run() {
    let _ = api::old::<u32>();
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("1 usages require manual migration");
    project.assert_file("src/lib.rs", source);
}

#[test]
fn arity_mismatch_does_not_modify_source() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", migrate = "old($x, $y) => new($x, $y)")]
    pub fn old(_: u32) {}
    pub fn new(_: u32, _: u32) {}
}
pub fn run() {
    api::old(1);
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_file("src/lib.rs", source);
}

#[test]
fn failed_compilation_prevents_all_edits() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
}
pub fn run() {
    api::old();
    missing();
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .failure()
        .stderr("cargo check must succeed");
    project.assert_file("src/lib.rs", source);
}

#[test]
fn nested_calls_can_be_migrated_in_successive_passes() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old(x: u32) -> u32 {
        x
    }
    pub fn new(x: u32) -> u32 {
        x
    }
}
pub fn run() -> u32 {
    api::old(api::old(1))
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("1 usages migrated")
        .stdout("1 usages require manual");
    project.assert_file(
        "src/lib.rs",
        &source.replace("api::old(api::old(1))", "api::new(api::old(1))"),
    );
    project.cargo(&["check"]).success();
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.assert_file("src/lib.rs", &source.replace("api::old(", "api::new("));
    project.cargo(&["check"]).success();
}

#[test]
fn anchored_replacement_keeps_its_written_scope() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "crate::new")]
    pub fn old() {}
}
pub fn new() {}
pub fn run() {
    api::old();
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project.cli(&["fix"]).success();
    project.assert_file("src/lib.rs", &source.replace("api::old()", "crate::new()"));
    project.cargo(&["check"]).success();
}
