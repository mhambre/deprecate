mod annotations;
mod arguments;
mod catalogs;
mod discovery;
mod migrations;
mod policy;
mod safety;

const MANIFEST: &str = r#"
[package]
name = "fixture"
version = "2.0.0"
edition = "2021"


[dependencies]
deprecate = { path = "$DEPRECATE" }
"#;
