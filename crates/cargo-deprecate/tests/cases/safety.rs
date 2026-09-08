use super::MANIFEST;
use crate::support::Project;

/// Checks both argument grouping and the replacement's surrounding expression.
#[test]
fn replacement_expression_preserves_outer_precedence() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", migrate = "old($x) => $x + 1")]
    pub fn old(x: i32) -> i32 {
        x + 1
    }
}
pub fn run() -> (i32, i32, i32) {
    (api::old(1) * 3, -api::old(2), 12 / api::old(1 + 2))
}
#[test]
fn behavior() {
    assert_eq!(run(), (6, -3, 3));
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project.cargo(&["test"]).success();
    project.cli(&["fix"]).success().stdout("3 usages migrated");
    project.assert_file(
        "src/lib.rs",
        &source
            .replace("api::old(1)", "((1) + 1)")
            .replace("api::old(2)", "((2) + 1)")
            .replace("api::old(1 + 2)", "((1 + 2) + 1)"),
    );
    project.cargo(&["test"]).success();
}

/// The local and external calls intentionally have the same diagnostic name.
#[test]
fn local_module_cannot_borrow_an_external_recipe() {
    let source = r"
pub mod dependency {
    pub mod api {
        #[deprecated]
        pub fn old() -> u32 {
            10
        }
        pub fn new() -> u32 {
            20
        }
    }
}
pub fn run() -> (u32, u32) {
    (dependency::api::old(), ::dependency::api::old())
}
#[test]
fn behavior() {
    assert_eq!(run(), (10, 1));
}
";
    let project = with_dependency(source);
    project.cargo(&["test"]).success();
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.assert_file(
        "src/lib.rs",
        &source.replace("::dependency::api::old()", "::dependency::api::new()"),
    );
    project.cargo(&["test"]).success();
}

/// Module aliases retain dependency ownership through Cargo's renamed edge.
#[test]
fn renamed_dependency_and_module_alias_resolve_together() {
    let source = r"
use renamed::api as legacy;
pub fn run() -> u32 {
    legacy::old()
}
";
    let project = with_dependency(source);
    project.write(
        "Cargo.toml",
        r#"
[package]
name = "consumer"
version = "1.0.0"
edition = "2021"

[dependencies]
renamed = { package = "dependency", path = "dependency" }
"#,
    );
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.assert_file(
        "src/lib.rs",
        &source.replace("legacy::old()", "legacy::new()"),
    );
    project.cargo(&["check"]).success();
}

/// A reexport's neighboring function need not be the original replacement.
#[test]
fn function_reexports_remain_manual() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() -> u32 {
        10
    }
    pub fn new() -> u32 {
        10
    }
}
pub mod facade {
    pub use crate::api::old;
    pub fn new() -> u32 {
        20
    }
}
pub fn run() -> u32 {
    facade::old()
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_unchanged();
}

/// Lexical item and generic bindings must not resolve through the extern prelude.
#[test]
fn block_and_generic_shadowing_remain_manual() {
    let source = r"
pub fn local() -> u32 {
    mod dependency {
        pub mod api {
            #[deprecated]
            pub fn old() -> u32 {
                10
            }
        }
    }
    dependency::api::old()
}
pub trait Api {
    #[deprecated]
    fn old() -> u32;
}
pub fn generic<dependency: Api>() -> u32 {
    dependency::old()
}
";
    let project = with_dependency(source);
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_unchanged();
}

/// Invalid Rust outside the declared module graph is not an input to migration.
#[test]
fn ignored_malformed_files_do_not_break_list_or_fix() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
}
pub fn run() {
    api::old();
}
"#;
    let broken = r"
fn incomplete(
";
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", source),
        ("src/unused.rs", broken),
        ("tests/ui/incomplete.rs", broken),
        ("target/generated/broken.rs", broken),
    ]);
    project.cli(&["list"]).success().stdout("usages      1");
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    for path in [
        "src/unused.rs",
        "tests/ui/incomplete.rs",
        "target/generated/broken.rs",
    ] {
        project.assert_file(path, broken);
    }
    project.cargo(&["check"]).success();
}

/// Changed evaluation counts and ordering require author review.
#[test]
fn reordered_repeated_and_dropped_arguments_remain_manual() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", migrate = "repeat($x) => new($x, $x)")]
    pub fn repeat(x: u32) -> u32 {
        x
    }
    #[deprecate::item(since = "1", migrate = "drop($x) => new(0, 0)")]
    pub fn drop(x: u32) -> u32 {
        x
    }
    #[deprecate::item(since = "1", migrate = "reorder($x, $y) => new($y, $x)")]
    pub fn reorder(x: u32, y: u32) -> u32 {
        x + y
    }
    pub fn new(x: u32, y: u32) -> u32 {
        x + y
    }
}
pub fn run() -> u32 {
    let mut count = 0;
    let mut next = || {
        count += 1;
        count
    };
    api::repeat(next());
    api::drop(next());
    api::reorder(next(), next());
    count
}
#[test]
fn behavior() {
    assert_eq!(run(), 4);
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("3 usages require manual migration");
    project.assert_unchanged();
    project.cargo(&["test"]).success();
}

/// Unknown helper paths cannot silently bind in the consumer's scope.
#[test]
fn multiple_relative_paths_and_control_flow_remain_manual() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", migrate = "old($x) => new(helper($x))")]
    pub fn old(x: bool) -> bool {
        x
    }
    #[deprecate::item(since = "1", migrate = "conditional($x, $y) => $x && $y")]
    pub fn conditional(x: bool, y: bool) -> bool {
        x && y
    }
    pub fn new(x: bool) -> bool {
        x
    }
    pub fn helper(x: bool) -> bool {
        x
    }
}
pub fn helper(_: bool) -> bool {
    false
}
pub fn run() -> bool {
    api::old(true) && api::conditional(true, true)
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("2 usages require manual migration");
    project.assert_unchanged();
}

/// One bad replacement restores every edited file, including successful edits.
#[test]
fn failed_replacement_compilation_restores_all_files() {
    let root = r#"
pub mod consumer;
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
    #[deprecate::item(since = "1", replacement = "missing")]
    pub fn broken() {}
}
pub fn run() {
    api::old();
}
"#;
    let consumer = r"
pub fn run() {
    crate::api::broken();
}
";
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", root),
        ("src/consumer.rs", consumer),
    ]);
    project.cli(&["fix", "--dry-run"]).success();
    project.assert_unchanged();
    project
        .cli(&["fix"])
        .failure()
        .stderr("restoring source files");
    project.assert_unchanged();
    project.cargo(&["check"]).success();
}

/// One physical file used in different module scopes has no single identity.
#[test]
fn reused_source_files_remain_manual() {
    let root = r#"
#[path = "shared.rs"]
pub mod first;
#[path = "shared.rs"]
pub mod second;
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
}
"#;
    let shared = r"
pub fn run() {
    crate::api::old();
}
";
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", root),
        ("src/shared.rs", shared),
    ]);
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_unchanged();
}

/// Package identity alone cannot distinguish library and binary declarations.
#[test]
fn library_recipe_cannot_match_a_binary_declaration() {
    let library = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() -> u32 {
        1
    }
    pub fn new() -> u32 {
        1
    }
}
pub fn run() -> u32 {
    api::old()
}
"#;
    let binary = r"
mod api {
    #[deprecated]
    pub fn old() -> u32 {
        10
    }
    pub fn new() -> u32 {
        20
    }
}
fn main() {
    assert_eq!(api::old(), 10);
}
";
    let project = Project::new(&[
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", library),
        ("src/main.rs", binary),
    ]);
    project.cli(&["fix"]).success().stdout("1 usages migrated");
    project.assert_file("src/main.rs", binary);
    project.assert_file("src/lib.rs", &library.replace("api::old()", "api::new()"));
    project.cargo(&["run"]).success();
}

/// Inactive declarations must not supply recipes for an active namesake.
#[test]
fn conditional_declarations_remain_manual() {
    let source = r#"
pub mod api {
    #[cfg(any())]
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() -> u32 {
        20
    }
    #[cfg(not(any()))]
    #[deprecated]
    pub fn old() -> u32 {
        10
    }
    pub fn new() -> u32 {
        20
    }
}
pub fn run() -> u32 {
    api::old()
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_unchanged();
}

/// Explicit local anchors and grouped module aliases preserve their scope.
#[test]
fn crate_self_super_and_grouped_alias_paths_migrate() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
}
pub mod consumer {
    use crate::api::{self as legacy};
    pub fn run() {
        crate::api::old();
        super::api::old();
        self::legacy::old();
    }
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project.cli(&["fix"]).success().stdout("3 usages migrated");
    project.assert_file(
        "src/lib.rs",
        &source
            .replace("api::old()", "api::new()")
            .replace("legacy::old()", "legacy::new()"),
    );
    project.cargo(&["check"]).success();
}

/// Macro-generated calls have no directly editable source expression.
#[test]
fn macro_generated_calls_remain_untouched() {
    let source = r"
macro_rules! invoke {
    () => {
        dependency::api::old()
    };
}
pub fn run() -> u32 {
    invoke!()
}
";
    let project = with_dependency(source);
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_unchanged();
}

/// Recursive path attributes produce a bounded error rather than endless scanning.
#[test]
fn recursive_module_paths_fail_without_modifying_files() {
    let source = r#"
#[path = "lib.rs"]
pub mod recursive;
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["list"])
        .failure()
        .stderr("recursive module source");
    project.assert_unchanged();
}

/// Rust 2015 absolute paths have different extern-prelude semantics.
#[test]
fn rust_2015_calls_remain_manual() {
    let source = r"
extern crate dependency;
pub fn run() -> u32 {
    ::dependency::api::old()
}
";
    let project = with_dependency(source);
    project.write(
        "Cargo.toml",
        &project.read("Cargo.toml").replace("2021", "2015"),
    );
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_file("src/lib.rs", source);
}

/// Declared modules outside the workspace are readable but never rewritten.
#[test]
fn external_module_files_are_not_edited() {
    let source = r#"
#[path = "../../shared.rs"]
pub mod shared;
pub mod api {
    #[deprecate::item(since = "1", replacement = "new")]
    pub fn old() {}
    pub fn new() {}
}
"#;
    let shared = r"
pub fn run() {
    crate::api::old();
}
";
    let project = Project::new(&[
        ("project/Cargo.toml", MANIFEST),
        ("project/src/lib.rs", source),
        ("shared.rs", shared),
    ]);
    project
        .cli_in("project", &["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_unchanged();
}

/// Identical replacements should not report a successful migration.
#[test]
fn self_replacement_does_not_report_an_edit() {
    let source = r#"
pub mod api {
    #[deprecate::item(since = "1", replacement = "old")]
    pub fn old() {}
}
pub fn run() {
    api::old();
}
"#;
    let project = Project::new(&[("Cargo.toml", MANIFEST), ("src/lib.rs", source)]);
    project
        .cli(&["fix"])
        .success()
        .stdout("No automatically migratable usages");
    project.assert_unchanged();
}

/// Supplies external metadata without making the dependency a workspace member.
fn with_dependency(source: &str) -> Project {
    Project::new(&[
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
        (
            "dependency/src/lib.rs",
            r"
pub mod api {
    #[deprecated]
    pub fn old() -> u32 {
        1
    }
    pub fn new() -> u32 {
        1
    }
}
",
        ),
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
            "name": "api::old",
            "since": "1",
            "replacement": "new"
        }
    ]
}
"#,
        ),
    ])
}
