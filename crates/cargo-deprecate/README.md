# cargo-deprecate

<p align="center">

[![CI](https://github.com/mhambre/deprecate/actions/workflows/ci.yml/badge.svg)](https://github.com/mhambre/deprecate/actions/workflows/ci.yml)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-blue)](https://github.com/mhambre/deprecate/actions/workflows/ci.yml)
![Crates.io Version](https://img.shields.io/crates/v/cargo-deprecate)

</p>

Inventory structured Rust deprecations, enforce removal deadlines in CI, and
apply supported call migrations from the `deprecate` annotation crate.

## Install and use

```console
cargo install cargo-deprecate
cargo deprecate list --dependencies
cargo deprecate check --dependencies --deny overdue
cargo deprecate fix --dry-run
cargo deprecate fix
```

Run commands inside a Cargo workspace. `list` shows lifecycle metadata and
candidate usage counts. `check` evaluates removal deadlines and replacement
policies, including deprecated Cargo features. Use `--help` on any subcommand
for its options.

## Migration safety

`fix` combines compiler deprecation spans with Cargo package identities and
explicit source bindings. It edits supported qualified calls in workspace
library and binary module files. Ambiguous ownership, function reexports,
generic or associated scopes, macro expansions, Rust 2015 crates, and templates
that change argument evaluation count or order remain manual.

A dry run previews edits without writing files or checking replacement types.
Applying edits reruns `cargo check --workspace` and restores edited sources if
compilation fails. Checks cover the current target and default feature selection;
they do not prove runtime equivalence. Review the plan and run your tests.

## Publishing metadata

`cargo deprecate catalog` writes `deprecations.json` for workspace packages.
`cargo deprecate diff --from previous.json` compares lifecycle metadata against
a saved catalog. Dependency catalogs allow discovery without relying on warnings
from dependency builds.

See the [annotation crate documentation](https://docs.rs/deprecate) and
[catalog format](https://github.com/mhambre/deprecate/blob/master/docs/catalog.md)
for authoring macros and migration recipes.
