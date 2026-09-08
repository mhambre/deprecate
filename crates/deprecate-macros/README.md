# deprecate-macros

<p align="center">

[![CI](https://github.com/mhambre/deprecate/actions/workflows/ci.yml/badge.svg)](https://github.com/mhambre/deprecate/actions/workflows/ci.yml)
![Crates.io Version](https://img.shields.io/crates/v/deprecate-macros)
![docs.rs](https://img.shields.io/docsrs/deprecate-macros)

</p>

Procedural macro implementation for [`deprecate`](https://crates.io/crates/deprecate).

Most users should depend on `deprecate`, which re-exports these macros. Install
the separate `cargo-deprecate` package for the `cargo deprecate` command.

[`item`](https://docs.rs/deprecate-macros/latest/deprecate_macros/attr.item.html)
adds standard compiler warnings, removal deadlines, replacement guidance, and
migration recipes. [`features!`](https://docs.rs/deprecate-macros/latest/deprecate_macros/macro.features.html)
records deprecated Cargo features for lifecycle checks.

Both macros require `since` and accept `remove`, `replacement`, `reason`, and
`migrate` as string-valued metadata. See each macro's API documentation for
syntax, validation rules, and runnable examples.
