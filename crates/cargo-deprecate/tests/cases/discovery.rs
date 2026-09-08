use super::MANIFEST;
use crate::support::Project;

/// Inline path overrides affect files, not the API's logical module name.
#[test]
fn inline_path_module_is_checked_cataloged_and_migrated() {
    let source = r#"
#[path = "actual"]
pub mod api {
    pub mod child;
}

pub fn run() {
    api::child::old();
}
"#;
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", source),
        ("src/actual/child.rs", INLINE_PATH_API),
    ]);
    assert_inline_path_commands(&project, "api::child::old");
    project.assert_file(
        "src/lib.rs",
        &source.replace("child::old()", "child::new()"),
    );
    project.assert_file("src/actual/child.rs", INLINE_PATH_API);
}

/// A path override on the first module reached from a file is relative to
/// that file's own directory, not its conventional module subdirectory.
/// Nested overrides then compose onto that corrected base like rustc does.
#[test]
fn nested_inline_paths_in_a_file_module_are_resolved() {
    let source = r"
pub mod outer;

pub fn run() {
    outer::api::nested::child::old();
}
";
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", source),
        (
            "src/outer.rs",
            r#"
#[path = "actual"]
pub mod api {
    #[path = "deeper"]
    pub mod nested {
        pub mod child;
    }
}
"#,
        ),
        ("src/actual/deeper/child.rs", INLINE_PATH_API),
    ]);
    assert_inline_path_commands(&project, "outer::api::nested::child::old");
    project.assert_file(
        "src/lib.rs",
        &source.replace("child::old()", "child::new()"),
    );
}

/// Inline overrides also compose with mod.rs and explicit child filenames.
#[test]
fn inline_path_in_directory_module_resolves_explicit_child_path() {
    let source = r"
pub mod outer;

pub fn run() {
    outer::api::child::old();
}
";
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", source),
        (
            "src/outer/mod.rs",
            r#"
#[path = "actual"]
pub mod api {
    #[path = "implementation.rs"]
    pub mod child;
}
"#,
        ),
        ("src/outer/actual/implementation.rs", INLINE_PATH_API),
    ]);
    assert_inline_path_commands(&project, "outer::api::child::old");
    project.assert_file(
        "src/lib.rs",
        &source.replace("child::old()", "child::new()"),
    );
}

const INLINE_PATH_API: &str = r#"
#[deprecate::item(since = "1", remove = "2", replacement = "new")]
pub fn old() {}

pub fn new() {}
"#;

/// Exercises the shared source graph through inventory, policy, catalog, and fix.
fn assert_inline_path_commands(project: &Project, name: &str) {
    project.cargo(&["check"]).success();
    project.cli(&["list"]).success().stdout(name);
    project
        .cli(&["check", "--deny", "overdue"])
        .failure()
        .stderr(name);
    project.cli(&["catalog"]).success();
    let catalog = project.json("deprecations.json");
    assert_eq!(catalog["deprecations"][0]["name"], name);
    project
        .cli(&["fix", "--dry-run"])
        .success()
        .stdout("1 migrations planned");
    project.assert_unchanged();
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.cargo(&["check"]).success();
}

#[test]
fn macro_imports_aliases_groups_and_globs_are_discovered() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
use deprecate::{self, features as feature_lifecycle, item};
#[item(since = "1", remove = "2")]
pub fn imported() {}
#[deprecate::item(since = "1", remove = "2")]
pub fn qualified() {}
pub mod nested {
    use deprecate as lifecycle;
    use lifecycle::item as retired;
    #[retired(since = "1", remove = "2")]
    pub fn aliased() {}
}
mod external;
"#,
        ),
        (
            "src/external.rs",
            r#"
use deprecate::*;
#[item(since = "1", remove = "2")]
pub fn globbed() {}
"#,
        ),
    ]);
    project.cargo(&["check"]).success();
    project
        .cli(&["check", "--deny", "overdue"])
        .failure()
        .stderr("4 deprecation policy violation(s)");
    project.cli(&["catalog"]).success();
    let catalog = project.json("deprecations.json");
    let names: Vec<_> = catalog["deprecations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "external::globbed",
            "imported",
            "nested::aliased",
            "qualified"
        ]
    );
}

#[test]
fn imports_are_scoped_to_their_module() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
mod a {
    use deprecate::item as retired;
    #[retired(since = "1", remove = "2")]
    pub fn old() {}
}
mod b {
    use deprecate::item as obsolete;
    #[obsolete(since = "1", remove = "2")]
    pub fn old() {}
}
"#,
        ),
    ]);
    project.cargo(&["check"]).success();
    project
        .cli(&["check", "--deny", "overdue"])
        .failure()
        .stderr("2 deprecation policy violation(s)");
    project
        .cli(&["list"])
        .success()
        .stdout("a::old")
        .stdout("b::old");
}

#[test]
fn block_local_imports_are_discovered() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r#"
pub fn outer() {
    use deprecate::item as retired;
    #[retired(since = "1", remove = "2")]
    fn local() {}
}
"#,
        ),
    ]);
    project.cargo(&["check"]).success();
    project
        .cli(&["check", "--deny", "overdue"])
        .failure()
        .stderr("1 deprecation policy violation(s)");
}

#[test]
fn custom_root_and_path_module_children_are_discovered() {
    let project = Project::new(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "custom"
version = "2.0.0"
edition = "2021"

[lib]
path = "lib.rs"

[dependencies]
deprecate = { path = "$DEPRECATE" }
"#,
        ),
        (
            "lib.rs",
            r#"
#[path = "alternate/mod.rs"]
pub mod api;
"#,
        ),
        (
            "alternate/mod.rs",
            r"
pub mod child;
",
        ),
        (
            "alternate/child.rs",
            r#"
#[deprecate::item(since = "1", remove = "2")]
pub fn old() {}
"#,
        ),
    ]);
    project.cargo(&["check"]).success();
    project
        .cli(&["check", "--deny", "overdue"])
        .failure()
        .stderr("api::child::old");
    project.cli(&["catalog"]).success();
    assert_eq!(
        project.json("deprecations.json")["deprecations"][0]["name"],
        "api::child::old"
    );
}

#[test]
fn conventional_file_and_directory_modules_keep_distinct_paths() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r"
pub mod a;
pub mod b;
",
        ),
        (
            "src/a.rs",
            r#"
#[deprecate::item(since = "1", remove = "2")]
pub fn old() {}
pub mod inner;
"#,
        ),
        (
            "src/a/inner.rs",
            r#"
#[deprecate::item(since = "1", remove = "2")]
pub fn old() {}
"#,
        ),
        (
            "src/b/mod.rs",
            r#"
#[deprecate::item(since = "1", remove = "2")]
pub fn old() {}
"#,
        ),
    ]);
    project.cargo(&["check"]).success();
    project
        .cli(&["list"])
        .success()
        .stdout("a::old")
        .stdout("a::inner::old")
        .stdout("b::old")
        .stdout("3 deprecations");
}

#[test]
fn unreferenced_source_and_build_output_are_not_declarations() {
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        (
            "src/lib.rs",
            r"
pub fn current() {}
",
        ),
        (
            "src/unreferenced.rs",
            r#"
#[deprecate::item(since = "1", remove = "2")]
pub fn old() {}
"#,
        ),
        (
            "target/generated.rs",
            r"
this is not Rust
",
        ),
    ]);
    project
        .cli(&["list"])
        .success()
        .stdout("No structured deprecations");
    project
        .cli(&["check", "--deny", "all"])
        .success()
        .stdout("(0 entries)");
}

#[test]
fn workspace_member_cwd_does_not_change_inventory() {
    let project = Project::new(&[
        (
            "Cargo.toml",
            r#"
[workspace]
members = ["one", "two"]
resolver = "2"
"#,
        ),
        (
            "one/Cargo.toml",
            r#"
[package]
name = "one"
version = "2.0.0"
edition = "2021"

[dependencies]
deprecate = { path = "$DEPRECATE" }
"#,
        ),
        (
            "one/src/lib.rs",
            r#"
#[deprecate::item(since = "1", remove = "2")]
pub fn old_one() {}
"#,
        ),
        (
            "two/Cargo.toml",
            r#"
[package]
name = "two"
version = "2.0.0"
edition = "2021"

[dependencies]
deprecate = { path = "$DEPRECATE" }
"#,
        ),
        (
            "two/src/lib.rs",
            r#"
#[deprecate::item(since = "1", remove = "2")]
pub fn old_two() {}
"#,
        ),
    ]);
    project
        .cli_in("one", &["list"])
        .success()
        .stdout("one: old_one")
        .stdout("two: old_two");
    project.cli(&["list"]).success().stdout("2 deprecations");
}

#[test]
fn same_dependency_name_at_two_versions_is_not_deduplicated() {
    let project = Project::new(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "consumer"
version = "1.0.0"
edition = "2021"

[dependencies]
older = { package = "dependency", path = "older" }
newer = { package = "dependency", path = "newer" }
"#,
        ),
        (
            "src/lib.rs",
            r"
pub fn current() {}
",
        ),
        (
            "older/Cargo.toml",
            r#"
[package]
name = "dependency"
version = "1.0.0"
edition = "2021"
"#,
        ),
        (
            "older/src/lib.rs",
            r"
pub fn old() {}
",
        ),
        (
            "older/deprecations.json",
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
            "remove": "2"
        }
    ]
}
"#,
        ),
        (
            "newer/Cargo.toml",
            r#"
[package]
name = "dependency"
version = "2.0.0"
edition = "2021"
"#,
        ),
        (
            "newer/src/lib.rs",
            r"
pub fn old() {}
",
        ),
        (
            "newer/deprecations.json",
            r#"
{
    "schema": 1,
    "crate": "dependency",
    "version": "2.0.0",
    "deprecations": [
        {
            "kind": "item",
            "name": "old",
            "since": "1",
            "remove": "2"
        }
    ]
}
"#,
        ),
    ]);
    project
        .cli(&["list"])
        .success()
        .stdout("No structured deprecations");
    project
        .cli(&["list", "--dependencies"])
        .success()
        .stdout("2 deprecations");
    project
        .cli(&["check", "--dependencies", "--deny", "overdue"])
        .failure()
        .stderr("1 deprecation policy violation(s)");
}
