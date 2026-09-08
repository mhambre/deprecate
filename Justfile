set dotenv-load := true
set default-list := true

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

test:
    cargo test --workspace --all-targets

lint:
    cargo clippy --workspace --all-targets -- -D warnings

docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
    cargo test --workspace --all-features --doc

check: fmt-check test lint docs

catalog:
    cargo run -p cargo-deprecate -- deprecate catalog

package:
    cargo package -p deprecate-macros --allow-dirty
    cargo package -p deprecate --allow-dirty --list
    cargo package -p cargo-deprecate --allow-dirty --list

publish-macros:
    cargo publish -p deprecate-macros

publish-main:
    cargo publish -p deprecate

publish-cli:
    cargo publish -p cargo-deprecate
