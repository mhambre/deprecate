# Contributing

Install stable Rust and [`just`](https://github.com/casey/just), then run:

```console
just check
```

Public behavior changes should include focused tests and corresponding user or
architecture documentation. Keep the public API small, dependencies minimal, and
source comments brief.

CLI behavior is tested with temporary projects defined as raw-string files. See
[the integration test guide](crates/cargo-deprecate/tests/README.md) for the shared
harness, assertions, and examples of adding cases.
