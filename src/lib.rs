#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

pub use deprecate_macros::{features, item};

/// The stable schema version emitted by `cargo deprecate catalog`.
pub const CATALOG_VERSION: u32 = 1;

/// Describes the kind of deprecated surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Kind {
    /// A Rust API item.
    Item,
    /// A Cargo feature.
    Feature,
}

/// Machine-readable lifecycle information for a deprecated API or feature.
///
/// Enable the `serde` Cargo feature for serialization and deserialization.
/// Constructing or deserializing this record does not validate its fields;
/// annotation macros and the Cargo command perform lifecycle validation.
///
/// # Examples
///
/// ```
/// use deprecate::{Deprecation, Kind};
///
/// let entry = Deprecation {
///     name: "api::old".into(),
///     kind: Kind::Item,
///     since: "1.2.0".into(),
///     remove: Some("2.0.0".into()),
///     replacement: Some("new".into()),
///     reason: None,
///     migrate: None,
/// };
/// assert_eq!(entry.kind, Kind::Item);
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Deprecation {
    /// The deprecated item path or feature name.
    pub name: String,
    /// Whether this describes an API item or Cargo feature.
    pub kind: Kind,
    /// The first crate version in which the surface was deprecated.
    pub since: String,
    /// The first crate version in which the surface may be removed.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub remove: Option<String>,
    /// The preferred replacement path or feature.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub replacement: Option<String>,
    /// A short explanation for the deprecation.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub reason: Option<String>,
    /// A call migration recipe such as `old($x) => new($x)`.
    ///
    /// Takes precedence over `replacement`; unsupported rewrites remain manual.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub migrate: Option<String>,
}
