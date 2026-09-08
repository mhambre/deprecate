#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

use proc_macro::TokenStream;
use proc_macro2::{Group, TokenStream as TokenStream2, TokenTree};
use quote::quote;
use semver::Version;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::{braced, Expr, ExprLit, Ident, Lit, LitStr, MetaNameValue, Token};

type Fields = BTreeMap<String, String>;

/// Adds a standard Rust deprecation and structured lifecycle metadata.
///
/// `since` is required. Optional fields are `remove`, `replacement`, `reason`,
/// and `migrate`; all values must be string literals. Lifecycle versions accept
/// abbreviated `SemVer` such as `2` or `2.1`. When present, `remove` must be later
/// than `since`. Unknown or duplicate fields produce a compile error.
///
/// `replacement` supplies guidance and a direct call replacement. `migrate`
/// takes precedence for automated migration and uses `old($arg) => new($arg)`
/// syntax. Valid recipes can still require manual migration when the Cargo
/// command cannot safely resolve or rewrite a call.
///
/// # Examples
///
/// ```
/// #[deprecate_macros::item(
///     since = "1.2",
///     remove = "2",
///     replacement = "new",
///     reason = "use the configurable implementation",
///     migrate = "old($value) => new($value)",
/// )]
/// pub fn old(value: u32) -> u32 {
///     new(value)
/// }
///
/// pub fn new(value: u32) -> u32 {
///     value
/// }
/// ```
#[proc_macro_attribute]
pub fn item(arguments: TokenStream, input: TokenStream) -> TokenStream {
    expand_item(arguments, input.clone()).unwrap_or_else(|error| error.to_compile_error().into())
}

/// Declares deprecated Cargo features for discovery by `cargo deprecate`.
///
/// Each feature name maps to the same lifecycle fields accepted by [`item`].
/// This macro records metadata; it neither defines Cargo features nor emits a
/// compiler warning when a feature is enabled. Declare the features in
/// `Cargo.toml` and use `cargo deprecate check --dependencies` to inspect the
/// resolved feature graph. Feature migrations are manual.
///
/// # Examples
///
/// ```
/// deprecate_macros::features! {
///     "legacy-tls" => {
///         since: "1.2",
///         remove: "2",
///         replacement: "rustls",
///         reason: "the legacy backend is no longer maintained",
///     },
/// }
/// ```
#[proc_macro]
pub fn features(input: TokenStream) -> TokenStream {
    expand_features(input).unwrap_or_else(|error| error.to_compile_error().into())
}

/// Produces standard deprecation and hidden metadata attributes for an item.
fn expand_item(arguments: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let fields = parse_fields(arguments)?;
    validate_fields(&fields)?;
    let name = item_name(&input)?;
    let since = fields.get("since").expect("validated fields include since");
    let compiler_since = parse_version(since)
        .expect("validated versions parse")
        .to_string();
    let note = build_note(&fields);
    let marker = marker("item", &name, &fields);

    let attributes = quote! {
        #[deprecated(since = #compiler_since, note = #note)]
        #[doc = #marker]
    };
    let mut output = TokenStream::from(attributes);
    output.extend(input);

    Ok(output)
}

/// Produces one hidden metadata constant for each feature declaration.
fn expand_features(input: TokenStream) -> syn::Result<TokenStream> {
    let declarations = syn::parse::<FeatureDeclarations>(input)?;
    let mut output = proc_macro::TokenStream::new();
    for declaration in declarations.entries {
        validate_fields(&declaration.fields)?;
        let marker = marker("feature", &declaration.name, &declaration.fields);
        output.extend(TokenStream::from(quote! {
            #[doc(hidden)]
            #[doc = #marker]
            const _: () = ();
        }));
    }
    Ok(output)
}

/// Parses the common comma-separated macro field syntax.
fn parse_fields(input: TokenStream) -> syn::Result<Fields> {
    let values = Punctuated::<MetaNameValue, Token![,]>::parse_terminated.parse(input)?;
    let mut fields = Fields::new();
    for value in values {
        let Some(key) = value.path.get_ident().map(ToString::to_string) else {
            return Err(syn::Error::new_spanned(value.path, "expected a field name"));
        };
        if !matches!(
            key.as_str(),
            "since" | "remove" | "replacement" | "reason" | "migrate"
        ) {
            return Err(syn::Error::new_spanned(
                value.path,
                format!("unknown deprecation field `{key}`"),
            ));
        }
        let Expr::Lit(ExprLit {
            lit: Lit::Str(literal),
            ..
        }) = value.value
        else {
            return Err(syn::Error::new_spanned(
                value.value,
                format!("expected a string literal for `{key}`"),
            ));
        };
        if fields.insert(key.clone(), literal.value()).is_some() {
            return Err(syn::Error::new_spanned(
                literal,
                format!("duplicate deprecation field `{key}`"),
            ));
        }
    }
    Ok(fields)
}

/// Validates lifecycle order and the minimum migration recipe shape.
fn validate_fields(fields: &Fields) -> syn::Result<()> {
    let since = fields.get("since").ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "missing required `since` field",
        )
    })?;
    let since_version = parse_version(since).map_err(|error| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`since` must be a semantic version: {error}"),
        )
    })?;
    if let Some(remove) = fields.get("remove") {
        let remove_version = parse_version(remove).map_err(|error| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`remove` must be a semantic version: {error}"),
            )
        })?;
        if remove_version <= since_version {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("`remove` ({remove}) must be later than `since` ({since})"),
            ));
        }
    }
    if let Some(recipe) = fields.get("migrate") {
        let Some((pattern, replacement)) = recipe.split_once("=>") else {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`migrate` must contain `pattern => replacement`",
            ));
        };
        if pattern.trim().is_empty() || replacement.trim().is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "both sides of `migrate` must be non-empty",
            ));
        }
        let pattern = syn::parse_str::<MigrationPattern>(pattern).map_err(|error| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("invalid migration pattern: {error}"),
            )
        })?;
        let parameters: BTreeSet<_> = pattern.parameters.iter().map(ToString::to_string).collect();
        if parameters.len() != pattern.parameters.len() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "migration pattern parameters must be unique",
            ));
        }
        let template = replacement.parse::<TokenStream2>().map_err(|error| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("invalid migration replacement: {error}"),
            )
        })?;
        let replacement = substitute_placeholders(template, &parameters)?;
        syn::parse2::<Expr>(replacement).map_err(|error| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                format!("the right side of `migrate` must be a Rust expression: {error}"),
            )
        })?;
    }
    Ok(())
}

struct MigrationPattern {
    parameters: Vec<Ident>,
}

impl Parse for MigrationPattern {
    /// Parses `path($name, ...)` without accepting arbitrary expressions.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<syn::Path>()?;
        let content;
        syn::parenthesized!(content in input);
        let mut parameters = Vec::new();
        while !content.is_empty() {
            content.parse::<Token![$]>()?;
            parameters.push(content.parse()?);
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after migration pattern"));
        }
        Ok(Self { parameters })
    }
}

/// Replaces declared placeholders so `syn` can validate the expression.
fn substitute_placeholders(
    tokens: TokenStream2,
    parameters: &BTreeSet<String>,
) -> syn::Result<TokenStream2> {
    let mut input = tokens.into_iter();
    let mut output = TokenStream2::new();
    while let Some(token) = input.next() {
        match token {
            TokenTree::Punct(punctuation) if punctuation.as_char() == '$' => {
                let Some(TokenTree::Ident(name)) = input.next() else {
                    return Err(syn::Error::new(
                        punctuation.span(),
                        "expected a placeholder name",
                    ));
                };
                if !parameters.contains(&name.to_string()) {
                    return Err(syn::Error::new(
                        name.span(),
                        "unknown migration placeholder",
                    ));
                }
                output.extend([TokenTree::Ident(Ident::new(
                    "__deprecate_argument",
                    name.span(),
                ))]);
            }
            TokenTree::Group(group) => {
                let mut replaced = Group::new(
                    group.delimiter(),
                    substitute_placeholders(group.stream(), parameters)?,
                );
                replaced.set_span(group.span());
                output.extend([TokenTree::Group(replaced)]);
            }
            token => output.extend([token]),
        }
    }
    Ok(output)
}

/// Accepts lifecycle shorthand before delegating validation to `semver`.
fn parse_version(value: &str) -> Result<Version, semver::Error> {
    let suffix_at = value.find(['-', '+']).unwrap_or(value.len());
    let (core, suffix) = value.split_at(suffix_at);
    let count = core.split('.').count();
    let normalized = match count {
        1 => format!("{core}.0.0{suffix}"),
        2 => format!("{core}.0{suffix}"),
        _ => value.to_owned(),
    };
    Version::parse(&normalized)
}

/// Builds the human-facing note used by Rust's standard warning.
fn build_note(fields: &Fields) -> String {
    let mut parts = Vec::new();
    if let Some(reason) = fields.get("reason") {
        parts.push(reason.clone());
    }
    if let Some(replacement) = fields.get("replacement") {
        parts.push(format!("use `{replacement}` instead"));
    }
    if let Some(remove) = fields.get("remove") {
        parts.push(format!("scheduled for removal in {remove}"));
    }
    parts.join("; ")
}

/// Serializes fields into the hidden rustdoc metadata marker.
fn marker(kind: &str, name: &str, fields: &Fields) -> String {
    let mut output = format!("deprecate:v1;kind={kind};name={}", encode(name));
    for (key, value) in fields {
        write!(output, ";{key}={}", encode(value)).expect("writing to a string cannot fail");
    }
    output
}

/// Percent-encodes bytes that could conflict with marker separators.
fn encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/') {
            output.push(char::from(byte));
        } else {
            write!(output, "%{byte:02X}").expect("writing to a string cannot fail");
        }
    }
    output
}

/// Locates a supported item name without changing the original token stream.
fn item_name(input: &TokenStream) -> syn::Result<String> {
    if let Ok(item) = syn::parse::<syn::Item>(input.clone()) {
        return item_ident(&item)
            .map(ToString::to_string)
            .ok_or_else(|| syn::Error::new_spanned(item, "expected a named Rust item"));
    }
    if let Ok(item) = syn::parse::<syn::ImplItem>(input.clone()) {
        return impl_item_ident(&item)
            .map(ToString::to_string)
            .ok_or_else(|| syn::Error::new_spanned(item, "expected a named implementation item"));
    }
    if let Ok(item) = syn::parse::<syn::TraitItem>(input.clone()) {
        return trait_item_ident(&item)
            .map(ToString::to_string)
            .ok_or_else(|| syn::Error::new_spanned(item, "expected a named trait item"));
    }
    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        "deprecate::item must be attached to a named Rust item",
    ))
}

/// Returns the identifier carried by a supported top-level item.
fn item_ident(item: &syn::Item) -> Option<&Ident> {
    match item {
        syn::Item::Const(item) => Some(&item.ident),
        syn::Item::Enum(item) => Some(&item.ident),
        syn::Item::Fn(item) => Some(&item.sig.ident),
        syn::Item::Mod(item) => Some(&item.ident),
        syn::Item::Static(item) => Some(&item.ident),
        syn::Item::Struct(item) => Some(&item.ident),
        syn::Item::Trait(item) => Some(&item.ident),
        syn::Item::TraitAlias(item) => Some(&item.ident),
        syn::Item::Type(item) => Some(&item.ident),
        syn::Item::Union(item) => Some(&item.ident),
        _ => None,
    }
}

/// Returns the identifier carried by a supported implementation item.
fn impl_item_ident(item: &syn::ImplItem) -> Option<&Ident> {
    match item {
        syn::ImplItem::Const(item) => Some(&item.ident),
        syn::ImplItem::Fn(item) => Some(&item.sig.ident),
        syn::ImplItem::Type(item) => Some(&item.ident),
        _ => None,
    }
}

/// Returns the identifier carried by a supported trait item.
fn trait_item_ident(item: &syn::TraitItem) -> Option<&Ident> {
    match item {
        syn::TraitItem::Const(item) => Some(&item.ident),
        syn::TraitItem::Fn(item) => Some(&item.sig.ident),
        syn::TraitItem::Type(item) => Some(&item.ident),
        _ => None,
    }
}

struct FeatureDeclarations {
    entries: Vec<FeatureDeclaration>,
}

impl Parse for FeatureDeclarations {
    /// Parses comma-separated feature declarations.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut entries = Vec::new();
        while !input.is_empty() {
            entries.push(input.parse()?);
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(Self { entries })
    }
}

struct FeatureDeclaration {
    name: String,
    fields: Fields,
}

impl Parse for FeatureDeclaration {
    /// Parses one `"feature" => { field: "value" }` declaration.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse::<LitStr>()?.value();
        input.parse::<Token![=>]>()?;
        let content;
        braced!(content in input);
        let mut fields = Fields::new();
        while !content.is_empty() {
            let key = content.parse::<Ident>()?;
            if !matches!(
                key.to_string().as_str(),
                "since" | "remove" | "replacement" | "reason" | "migrate"
            ) {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown deprecation field `{key}`"),
                ));
            }
            content.parse::<Token![:]>()?;
            let value = content.parse::<LitStr>()?;
            if fields.insert(key.to_string(), value.value()).is_some() {
                return Err(syn::Error::new(key.span(), "duplicate deprecation field"));
            }
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }
        Ok(Self { name, fields })
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_version, validate_fields, Fields};

    #[test]
    fn accepts_abbreviated_and_full_versions() {
        assert_eq!(parse_version("2").expect("version").to_string(), "2.0.0");
        assert_eq!(parse_version("2.1").expect("version").to_string(), "2.1.0");
        assert!(parse_version("2.1.0-beta.1+build.5").is_ok());
    }

    #[test]
    fn rejects_invalid_versions() {
        assert!(parse_version("01.2.3").is_err());
        assert!(parse_version("1.2.3-").is_err());
        assert!(parse_version("1.2.3-beta_1").is_err());
    }

    #[test]
    fn orders_prereleases_before_releases() {
        let beta = parse_version("2.0.0-beta.1").expect("prerelease");
        let release = parse_version("2.0.0").expect("release");
        assert!(beta < release);
    }

    #[test]
    fn rejects_unknown_migration_placeholder() {
        let fields = Fields::from([
            ("since".to_owned(), "1".to_owned()),
            (
                "migrate".to_owned(),
                "old($value) => New::from($other)".to_owned(),
            ),
        ]);
        let error = validate_fields(&fields).expect_err("placeholder should be rejected");
        assert!(error.to_string().contains("unknown migration placeholder"));
    }
}
