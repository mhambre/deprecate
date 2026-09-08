# CLI integration tests

Each case defines its project as a list of `(relative_path, contents)` pairs.
Use raw strings for Rust, TOML, and JSON so the fixture reads like real files.
Start the contents on the line after the opening delimiter, align top-level
contents at the left margin, and format bodies and JSON across multiple lines.
The harness creates directories and substitutes the quoted `"$DEPRECATE"` path
with the local annotation crate's absolute path. No brace escaping is needed.

```rust,ignore
use crate::support::Project;

#[test]
fn overdue_api_fails_policy() {
    let project = Project::new(&[
        ("Cargo.toml", r#"
[package]
name = "example"
version = "2.0.0"
edition = "2021"

[dependencies]
deprecate = { path = "$DEPRECATE" }
"#),
        ("src/lib.rs", r#"
pub mod api;
"#),
        ("src/api.rs", r#"
#[deprecate::item(since = "1", remove = "2")]
pub fn old() {}
"#),
    ]);

    project.cargo(&["check"]).success();
    project.cli(&["check", "--deny", "overdue"])
        .failure()
        .stderr("deprecate::removal_due");
    project.assert_unchanged();
}
```

Simple cases can reuse `super::MANIFEST`. Workspaces, path dependencies, custom
targets, and feature configurations should define their own complete manifests.

## Assertions

- `cli(args)` runs the built executable; `cli_in(directory, args)` changes its
  working directory. Include `"deprecate"` as the first argument to test Cargo's
  subcommand invocation shape.
- `cargo(args)` runs Cargo against the fixture. After applying migrations, use
  `cargo(&["check"])` or `cargo(&["test"])` to verify compilation or behavior.
- Command results support `success()`, `failure()`, `stdout(text)`, `stderr(text)`,
  and `stdout_excludes(text)`. Failures show the command, directory, exit status,
  stdout, and stderr.
- `read(path)`, `write(path, contents)`, and `assert_file(path, contents)` support
  before/after scenarios. `assert_unchanged()` compares every originally defined
  file against its initial contents; generated lockfiles and catalogs are excluded.
- `json(path)` parses a generated catalog for structural assertions.

Each project has a temporary directory and its own Cargo target directory, both
removed when the case finishes. Fixture Cargo commands run offline, so additional
registry dependencies must already be cached. Prefer local path dependencies for
dependency-resolution scenarios. Rust flags are cleared so intentional warnings
are not promoted to errors by the caller's environment.

## Adding and running cases

Add tests under `cases/`: `migrations`, `discovery`, `policy`, `catalogs`,
`annotations`, `arguments`, or `safety`. They share one integration-test binary and harness.
Use separate named tests for independent behaviors so failures can be rerun alone.
Assert source preservation for manual cases and dry runs, exact edits for fixes,
and compiler success for fixtures intended to represent valid Rust.

Safety regressions also compare runtime results before and after precedence and
ownership fixes, and check that failed replacement compilation restores all files.

```console
cargo test -p cargo-deprecate --test cli
cargo test -p cargo-deprecate --test cli cases::migrations
cargo test -p cargo-deprecate --test cli module_alias -- --nocapture
```
