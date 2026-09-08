#![allow(deprecated, missing_docs)]

#[deprecate::item(
    since = "1.0",
    remove = "2",
    replacement = "new_api",
    reason = "the new API returns a typed value",
    migrate = "old_api($value) => new_api($value)"
)]
fn old_api(value: u32) -> u32 {
    value
}

deprecate::features! {
    "old-feature" => {
        since: "1",
        remove: "2",
        replacement: "new-feature",
    },
}

#[test]
fn annotated_item_keeps_its_behavior() {
    assert_eq!(old_api(42), 42);
}
