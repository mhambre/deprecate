# deprecate-macros

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
