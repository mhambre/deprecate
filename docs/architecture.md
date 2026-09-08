# Architecture

The project separates author annotations, published metadata, and consumer
operations so each layer remains useful on stable Rust.

The `deprecate-macros` crate validates annotations and emits the standard
`#[deprecated]` attribute. It also emits hidden schema v1 metadata for future
tooling. Feature declarations use the same metadata format.

The `deprecate` library re-exports both macros and owns the public catalog model.
Its serialization support is optional. The separate `cargo-deprecate` package owns
all command and migration dependencies.

The `cargo-deprecate` executable asks Cargo for its resolved metadata, scans Rust
sources for structured declarations, and correlates feature declarations with the
resolved feature set. This avoids relying on warnings emitted while dependencies
compile, which Cargo may suppress.

Shared CLI helpers own version normalization, recipe parsing, and
output formatting. Discovery and migration parse Rust syntax before editing.

Catalog files are the long-term exchange boundary. Source scanning permits
adoption before every crate publishes a catalog. A future registry service or
compiler integration can consume the same schema without changing author syntax.

Migration recipes intentionally describe calls. Before editing, the command reads
Rust's JSON diagnostics from `cargo check` and limits candidates to primary source
spans marked deprecated. Diagnostic display names are not ownership evidence.
The resolver follows explicit module bindings and Cargo dependency aliases to an
exact package ID and declaration path. Unknown or ambiguous bindings remain manual.

Discovery and migration share Cargo crate roots and declared module files. Cycles
are rejected, and files reused across module contexts are not automatically edited.
Replacements group compound expressions and preserve placeholder count and order.
Unsupported control flow and ambiguous helper paths require manual migration.

Before applying edits, the engine compares each source with its planning snapshot.
After applying, it reruns `cargo check --workspace` and restores edits on failure,
reporting restoration errors rather than hiding them. Validation covers the current
Cargo configuration, not all features or runtime behavior. Dry runs only plan edits.
