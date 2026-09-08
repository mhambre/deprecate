#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod catalog;
mod commands;
mod helpers;
mod migrate;
mod resolve;
mod scan;
mod workspace;

/// Reports command failures with a nonzero exit status.
fn main() {
    if let Err(error) = commands::run(std::env::args().skip(1).collect()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
