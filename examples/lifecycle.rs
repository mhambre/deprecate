#![allow(missing_docs)]

#[deprecate::item(
    since = "1.4.0",
    remove = "2.0.0",
    replacement = "milliseconds_v2",
    reason = "the new name follows the crate naming convention",
    migrate = "milliseconds($n) => milliseconds_v2($n)"
)]
fn milliseconds(n: u64) -> std::time::Duration {
    std::time::Duration::from_millis(n)
}

fn milliseconds_v2(n: u64) -> std::time::Duration {
    std::time::Duration::from_millis(n)
}

deprecate::features! {
    "old-tls" => {
        since: "1.4.0",
        remove: "2.0.0",
        replacement: "rustls",
    },
}

fn main() {
    #[cfg_attr(clippy, allow(deprecated))]
    let old = milliseconds(10);
    assert_eq!(old, milliseconds_v2(10));
}
