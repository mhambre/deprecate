# deprecate

<p align="center">

[![CI](https://github.com/mhambre/deprecate/actions/workflows/ci.yml/badge.svg)](https://github.com/mhambre/deprecate/actions/workflows/ci.yml)

</p>

Rust deprecation macros for identifiers and features with removal deadlines,
replacement guidance, and migration recipes. Deprecate APIs and Cargo features,
automate supported call migrations, and check lifecycle policy in CI.

## Annotating an API

```rust,ignore
#[deprecate::item(
    since = "2.1.0",
    remove = "3.0.0",
    replacement = "Client::builder",
    reason = "client construction is now configurable",
    migrate = "new_client($url) => Client::builder().url($url).build()",
)]
pub fn new_client(url: Url) -> Client {
    value
}
```

Calling `new_client` now produces a standard compiler warning for the user to either manually
migrate, or user `cargo deprecate fix` to automatically rewrite the call to the new API:

```text
warning: use of deprecated function `new_client`: client construction is now configurable; replace `new_client(url)` with `Client::builder().url(url).build()`; scheduled for removal in 3.0.0
  --> src/lib.rs:24:5
   |
24 |     new_client(url)
   |     ^^^^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default
```

The macro validates versions with the `semver` crate and requires `remove` to be
later than `since`. It accepts abbreviated lifecycle versions such as `2` and
`2.1`, normalizing them to `2.0.0` and `2.1.0` before comparison.

Source discovery recognizes locally imported macros such as `use deprecate::item`
and aliases such as `use deprecate::item as retired`, including grouped imports.

## Deprecation lifecycle

`since`, `remove`, and the package's current version use the same `SemVer` ordering.
The macro always requires `remove > since`. During `cargo deprecate check`, an API
is active while `since <= current < remove` and is overdue once
`current >= remove`. Omitting `remove` leaves the deprecation active without a
scheduled deadline. A prerelease such as `3.0.0-beta.1` remains earlier than the
`3.0.0` removal boundary.

## Installation

Library authors add the annotation crate:

```console
cargo add deprecate
```

Developers and CI install the separate Cargo command package:

```console
cargo install cargo-deprecate
```

## Cargo command

```console
cargo deprecate
cargo deprecate --workspace
cargo deprecate list --dependencies
cargo deprecate check --dependencies --deny overdue
cargo deprecate check --deny missing-replacement
cargo deprecate fix --dry-run
cargo deprecate fix
cargo deprecate catalog
cargo deprecate diff --from deprecations-v1.7.json
```

For graph-wide inventory and checks, `--workspace` and `--dependencies` both
include resolved dependencies. Without either flag, `list` and `check` inspect the
workspace's own packages.

## Migrating calls

`fix` combines Cargo dependency identities, explicit source bindings, and compiler
deprecation spans to edit supported calls in workspace library and binary targets.
Explicit migration recipes bind call arguments by name. A recipe such as
`old($value) => New::from($value)` can transform `api::old(input)` into
`api::New::from(input)`.
A plain replacement performs a direct call-path rewrite. Replacement paths in an
explicit recipe are relative to the qualifier used for the deprecated call. For
example, `dependency::old(value)` with `old($x) => new($x)` becomes
`dependency::new(value)`. Placeholders substituted next to an operator, cast, or
postfix chain (such as `$x + 1` or `$x.method()`) are parenthesized so the
argument's own precedence can't merge into the template; placeholders that occupy
a whole call, array, or tuple argument are left bare. Fully anchored `::`,
`crate`, `self`, and `super` paths retain their written meaning. The command
skips declarations, comments, strings, arity mismatches, generic calls,
unresolved declarations, and overlapping edits.
Bare imported calls, function reexports, ambiguous scopes, Rust 2015 crates,
associated or generic scopes, and shared source files remain manual. Templates
that repeat, drop, reorder, or conditionally evaluate arguments also remain manual.
Only declared module files are read; unrelated Rust fixtures are ignored.

Use `--dry-run` to review a plan without editing or validating the replacement's
types. Applying a plan reruns `cargo check --workspace` and restores edited files
if compilation fails. This does not prove behavioral equivalence: review recipes
and run your tests. Checks use the current target and default feature selection,
not every possible configuration. `list` counts syntactic candidates; manual
counts from `fix` can include deprecated imports as well as calls.

## Library features

The library re-exports `item` and `features!` and provides the `Deprecation` and
`Kind` catalog types. Enable the optional `serde` feature to serialize those types.
The Cargo command is installed separately and is not a library feature.

## Deprecated Cargo features

Cargo does not reliably surface dependency build warnings. Feature declarations
are therefore discovered from the resolved graph by `cargo deprecate check`.

```rust
deprecate::features! {
    "old-tls" => {
        since: "2.4",
        remove: "3",
        replacement: "rustls",
    },
    "legacy-runtime" => {
        since: "2.0",
        remove: "3",
        reason: "the compatibility runtime is no longer maintained",
    },
}
```

## Migration catalogs

`cargo deprecate catalog` writes `deprecations.json` beside each workspace
manifest. The versioned format is designed to ship in crate packages and remain
readable by tooling without Rust compiler internals. See
[`docs/catalog.md`](https://github.com/mhambre/deprecate/blob/master/docs/catalog.md)
for the schema and compatibility rules.

Dependency catalogs are preferred when present. Source discovery is the stable
fallback for crates that have not published one yet. Catalogs become the durable
contract for registries, documentation sites, release tooling, and future compiler
integrations.

## MSRV

The minimum supported Rust version is 1.74. The annotation crate keeps CLI
dependencies in the separate `cargo-deprecate` package.
