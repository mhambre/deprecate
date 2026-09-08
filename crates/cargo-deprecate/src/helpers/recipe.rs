use deprecate::Deprecation;
use proc_macro2::{Group, TokenStream, TokenTree};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{parenthesized, Expr, Ident, Path, Token};

/// A validated call pattern and replacement template.
pub(crate) struct Recipe {
    pattern: Vec<String>,
    parameters: Vec<String>,
    replacement: String,
    qualify_at: Option<usize>,
    explicit: bool,
    automatic: bool,
}

impl Recipe {
    /// Builds an explicit recipe or a direct path replacement.
    pub(crate) fn from_deprecation(data: &Deprecation) -> Result<Option<Self>, String> {
        if let Some(recipe) = &data.migrate {
            return Self::explicit(data, recipe).map(Some);
        }
        let declared = split_path(&data.name);
        if declared.is_empty() {
            return Err("deprecated item name must be a Rust path".to_owned());
        }
        Ok(data.replacement.as_ref().map(|replacement| Self {
            pattern: declared,
            parameters: Vec::new(),
            replacement: replacement.clone(),
            qualify_at: None,
            explicit: false,
            automatic: true,
        }))
    }

    /// Renders a migration after checking call arity and bindings.
    pub(crate) fn render(&self, call: &[String], arguments: &[&str]) -> Option<String> {
        if !self.automatic
            || (self.explicit && self.pattern.len() > 1 && !call.ends_with(&self.pattern))
        {
            return None;
        }
        if !self.explicit {
            return self.render_direct(call, arguments);
        }
        if self.parameters.len() != arguments.len() {
            return None;
        }
        let raw: BTreeMap<_, _> = self
            .parameters
            .iter()
            .zip(arguments)
            .map(|(parameter, argument)| (parameter.as_str(), argument.trim().to_owned()))
            .collect();
        let wrapped: BTreeMap<_, _> = raw
            .iter()
            .map(|(name, value)| (*name, format!("({value})")))
            .collect();
        let template = TokenStream::from_str(&self.replacement).ok()?;
        let mut ranges = Vec::new();
        placeholder_ranges(template, &raw, &wrapped, &mut ranges)?;
        ranges.sort_by_key(|(range, _)| range.start);
        let mut output = self.replacement.clone();
        for (range, value) in ranges.iter().rev() {
            output.replace_range(range.clone(), value);
        }
        if let Some(offset) = self.qualify_at {
            let prefix = self.external_prefix(call);
            if !prefix.is_empty() {
                output.insert_str(
                    adjusted_offset(offset, &ranges),
                    &format!("{}::", prefix.join("::")),
                );
            }
        }
        let expression = syn::parse_str::<Expr>(&output).ok()?;
        Some(
            if matches!(
                expression,
                Expr::Call(_) | Expr::MethodCall(_) | Expr::Path(_) | Expr::Paren(_) | Expr::Lit(_)
            ) {
                output
            } else {
                format!("({output})")
            },
        )
    }

    /// Parses and validates an explicit recipe from metadata.
    fn explicit(data: &Deprecation, recipe: &str) -> Result<Self, String> {
        let (pattern, replacement) = recipe
            .split_once("=>")
            .ok_or_else(|| "migration recipe must contain `pattern => replacement`".to_owned())?;
        let pattern = syn::parse_str::<CallPattern>(pattern)
            .map_err(|error| format!("invalid migration pattern: {error}"))?;
        let pattern_path = path_segments(&pattern.path);
        let parameters: Vec<_> = pattern
            .parameters
            .into_iter()
            .map(|parameter| parameter.to_string())
            .collect();
        let unique: BTreeSet<_> = parameters.iter().collect();
        if unique.len() != parameters.len() {
            return Err("migration pattern parameters must be unique".to_owned());
        }
        let template = TokenStream::from_str(replacement.trim())
            .map_err(|error| format!("invalid migration replacement: {error}"))?;
        let mut bindings = Vec::new();
        binding_order(template.clone(), &mut bindings);
        let placeholders: BTreeMap<_, _> = parameters
            .iter()
            .map(|parameter| {
                (
                    parameter.as_str(),
                    TokenStream::from_str("__deprecate_argument")
                        .expect("a fixed identifier is valid tokens"),
                )
            })
            .collect();
        let expression = substitute(template, &placeholders)
            .ok_or_else(|| "migration replacement uses an unknown placeholder".to_owned())?;
        let expression = syn::parse2::<Expr>(expression)
            .map_err(|error| format!("migration replacement must be a Rust expression: {error}"))?;
        let qualify_at = replacement_path(&expression);
        let mut safety = TemplateSafety {
            supported: true,
            relative_paths: Vec::new(),
        };
        safety.visit_expr(&expression);
        let automatic = bindings == parameters
            && !replacement.contains("__deprecate_argument")
            && safety.supported
            && (safety.relative_paths.is_empty()
                || safety.relative_paths == qualify_at.into_iter().collect::<Vec<_>>());
        let declared = split_path(&data.name);
        if declared.last() != pattern_path.last() {
            return Err("migration pattern must target the deprecated item".to_owned());
        }
        Ok(Self {
            pattern: pattern_path,
            parameters,
            replacement: replacement.trim().to_owned(),
            qualify_at,
            explicit: true,
            automatic,
        })
    }

    /// Uses the caller's complete parent path for a direct replacement.
    fn render_direct(&self, call: &[String], arguments: &[&str]) -> Option<String> {
        call.len().checked_sub(1)?;
        let caller = &call[..call.len() - 1];
        let path = if caller.is_empty() || is_anchored(&self.replacement) {
            self.replacement.clone()
        } else {
            format!("{}::{}", caller.join("::"), self.replacement)
        };
        Some(format!("{path}({})", arguments.join(",")))
    }

    /// Removes the recipe path while retaining downstream qualifiers.
    fn external_prefix<'a>(&self, call: &'a [String]) -> &'a [String] {
        if call.ends_with(&self.pattern) {
            &call[..call.len() - self.pattern.len()]
        } else {
            &call[..call.len().saturating_sub(1)]
        }
    }
}

struct CallPattern {
    path: Path,
    parameters: Vec<Ident>,
}

/// Records argument evaluation order, including repeated placeholders.
fn binding_order(tokens: TokenStream, names: &mut Vec<String>) {
    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Punct(punctuation) if punctuation.as_char() == '$' => {
                if let Some(TokenTree::Ident(name)) = tokens.next() {
                    names.push(name.to_string());
                }
            }
            TokenTree::Group(group) => binding_order(group.stream(), names),
            _ => {}
        }
    }
}

struct TemplateSafety {
    supported: bool,
    relative_paths: Vec<usize>,
}

impl<'ast> Visit<'ast> for TemplateSafety {
    /// Rejects templates whose control flow can skip argument evaluation.
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if !matches!(
            expression,
            Expr::Call(_)
                | Expr::MethodCall(_)
                | Expr::Path(_)
                | Expr::Paren(_)
                | Expr::Group(_)
                | Expr::Unary(_)
                | Expr::Binary(_)
                | Expr::Cast(_)
                | Expr::Index(_)
                | Expr::Field(_)
                | Expr::Reference(_)
                | Expr::Tuple(_)
                | Expr::Array(_)
                | Expr::Lit(_)
        ) {
            self.supported = false;
        }
        if let Expr::Binary(binary) = expression {
            if matches!(binary.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) {
                self.supported = false;
            }
        }
        syn::visit::visit_expr(self, expression);
    }

    /// Collects paths whose meaning would change without qualification.
    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        if expression.qself.is_some() {
            self.supported = false;
        }
        if let Some(offset) = relative_path(&expression.path) {
            self.relative_paths.push(offset);
        }
        syn::visit::visit_expr_path(self, expression);
    }

    /// Generic substitutions require type resolution beyond template rendering.
    fn visit_path(&mut self, path: &'ast Path) {
        if path
            .segments
            .iter()
            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
        {
            self.supported = false;
        }
        syn::visit::visit_path(self, path);
    }

    /// Unqualified type aliases in casts could bind to a consumer's type.
    fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
        let primitive = ty.qself.is_none()
            && ty.path.leading_colon.is_none()
            && ty.path.segments.len() == 1
            && ty.path.segments.first().is_some_and(|segment| {
                matches!(
                    segment.ident.to_string().as_str(),
                    "u8" | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "usize"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "isize"
                        | "f32"
                        | "f64"
                        | "bool"
                        | "char"
                )
            });
        self.supported &= primitive;
        syn::visit::visit_type_path(self, ty);
    }
}

impl Parse for CallPattern {
    /// Parses `path($name, ...)` without arbitrary expressions.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse()?;
        let content;
        parenthesized!(content in input);
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
        Ok(Self { path, parameters })
    }
}

/// Replaces placeholders recursively for syntax validation.
fn substitute(tokens: TokenStream, bindings: &BTreeMap<&str, TokenStream>) -> Option<TokenStream> {
    let mut input = tokens.into_iter();
    let mut output = TokenStream::new();
    while let Some(token) = input.next() {
        match token {
            TokenTree::Punct(punctuation) if punctuation.as_char() == '$' => {
                let TokenTree::Ident(name) = input.next()? else {
                    return None;
                };
                output.extend(bindings.get(name.to_string().as_str())?.clone());
            }
            TokenTree::Group(group) => {
                let mut replaced =
                    Group::new(group.delimiter(), substitute(group.stream(), bindings)?);
                replaced.set_span(group.span());
                output.extend([TokenTree::Group(replaced)]);
            }
            token => output.extend([token]),
        }
    }
    Some(output)
}

/// Locates placeholders without inspecting literal contents.
///
/// A placeholder that occupies a whole, delimiter-bounded position (a
/// standalone call/array/tuple argument, bounded by the start/end of its
/// group or by commas) is already fully protected by Rust's grammar and
/// uses the raw argument text. Any other position - adjacent to an
/// operator, a field/method access, or a cast - keeps the parenthesized
/// form so the argument's own precedence can't leak into the template.
fn placeholder_ranges(
    tokens: TokenStream,
    raw: &BTreeMap<&str, String>,
    wrapped: &BTreeMap<&str, String>,
    ranges: &mut Vec<(std::ops::Range<usize>, String)>,
) -> Option<()> {
    let mut input = tokens.into_iter().peekable();
    let mut at_boundary = true;
    while let Some(token) = input.next() {
        if let TokenTree::Punct(punctuation) = &token {
            if punctuation.as_char() == '$' {
                let start = punctuation.span().byte_range().start;
                let TokenTree::Ident(name) = input.next()? else {
                    return None;
                };
                let end = name.span().byte_range().end;
                let bounded = at_boundary && is_boundary(input.peek());
                let bindings = if bounded { raw } else { wrapped };
                let value = bindings.get(name.to_string().as_str())?.clone();
                ranges.push((start..end, value));
                at_boundary = false;
                continue;
            }
            if punctuation.as_char() == ',' {
                at_boundary = true;
                continue;
            }
        }
        if let TokenTree::Group(group) = &token {
            placeholder_ranges(group.stream(), raw, wrapped, ranges)?;
        }
        at_boundary = false;
    }
    Some(())
}

/// Reports whether a peeked token ends a comma-delimited element.
fn is_boundary(token: Option<&TokenTree>) -> bool {
    match token {
        None => true,
        Some(TokenTree::Punct(punctuation)) => punctuation.as_char() == ',',
        _ => false,
    }
}

/// Adjusts a byte offset after earlier placeholder expansion.
fn adjusted_offset(offset: usize, ranges: &[(std::ops::Range<usize>, String)]) -> usize {
    ranges
        .iter()
        .filter(|(range, _)| range.start < offset)
        .fold(offset, |offset, (range, value)| {
            if value.len() >= range.len() {
                offset
                    .checked_add(value.len() - range.len())
                    .expect("replacement offset fits into usize")
            } else {
                offset
                    .checked_sub(range.len() - value.len())
                    .expect("replacement offset remains non-negative")
            }
        })
}

/// Finds the package-relative path that starts a replacement.
fn replacement_path(expression: &Expr) -> Option<usize> {
    let path = match expression {
        Expr::Call(call) => return replacement_path(&call.func),
        Expr::MethodCall(call) => return replacement_path(&call.receiver),
        Expr::Await(expression) => return replacement_path(&expression.base),
        Expr::Group(expression) => return replacement_path(&expression.expr),
        Expr::Paren(expression) => return replacement_path(&expression.expr),
        Expr::Try(expression) => return replacement_path(&expression.expr),
        Expr::Path(path) => &path.path,
        _ => return None,
    };
    relative_path(path)
}

/// Finds paths that need the caller's module prefix.
fn relative_path(path: &Path) -> Option<usize> {
    let first = path.segments.first()?.ident.to_string();
    if path.leading_colon.is_some()
        || matches!(
            first.as_str(),
            "crate" | "self" | "super" | "__deprecate_argument"
        )
    {
        None
    } else {
        Some(path.span().byte_range().start)
    }
}

fn path_segments(path: &Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn split_path(path: &str) -> Vec<String> {
    path.split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn is_anchored(path: &str) -> bool {
    path.starts_with("::")
        || path.starts_with("crate::")
        || path.starts_with("self::")
        || path.starts_with("super::")
}

#[cfg(test)]
mod tests {
    use super::Recipe;
    use deprecate::{Deprecation, Kind};

    /// Builds the smallest entry needed by recipe tests.
    fn deprecation(name: &str, recipe: &str) -> Deprecation {
        Deprecation {
            name: name.to_owned(),
            kind: Kind::Item,
            since: "1".to_owned(),
            remove: None,
            replacement: None,
            reason: None,
            migrate: Some(recipe.to_owned()),
        }
    }

    #[test]
    fn preserves_precedence_and_placeholder_boundaries() {
        let recipe =
            Recipe::from_deprecation(&deprecation("old", "old($x, $xy) => new($x * 2, $xy)"))
                .expect("recipe is valid")
                .expect("recipe exists");
        assert_eq!(
            recipe.render(&["old".to_owned()], &["1 + 2", "value"]),
            Some("new((1 + 2) * 2, value)".to_owned())
        );
    }

    #[test]
    fn retains_downstream_qualifier() {
        let recipe = Recipe::from_deprecation(&deprecation("old", "old($x) => new($x)"))
            .expect("recipe is valid")
            .expect("recipe exists");
        assert_eq!(
            recipe.render(&["dependency".to_owned(), "old".to_owned()], &["1"]),
            Some("dependency::new(1)".to_owned())
        );
    }

    /// A placeholder used as a whole argument never needs protection, but one
    /// embedded in an operator, cast, or postfix chain always does, since the
    /// argument's own precedence could otherwise merge into the template.
    #[test]
    fn wraps_only_placeholders_exposed_to_an_operator_or_postfix_chain() {
        let recipe = Recipe::from_deprecation(&deprecation(
            "old",
            "old($x, $y, $z) => new($x, $y.trailing_zeros(), $z as u64)",
        ))
        .expect("recipe is valid")
        .expect("recipe exists");
        assert_eq!(
            recipe.render(&["old".to_owned()], &["1 + 2", "3 + 4", "5 | 6"]),
            Some("new(1 + 2, (3 + 4).trailing_zeros(), (5 | 6) as u64)".to_owned())
        );
    }

    #[test]
    fn rejects_unknown_placeholders() {
        let error = Recipe::from_deprecation(&deprecation("old", "old($x) => New::from($y)"))
            .err()
            .expect("recipe is rejected");
        assert!(error.contains("unknown placeholder"));
    }

    /// Valid syntax does not imply a template is safe to automate.
    #[test]
    fn unsupported_evaluation_and_resolution_templates_remain_manual() {
        for template in [
            "old($x, $y) => new($x?, $y)",
            "old($x, $y) => new($x.await, $y)",
            "old($x, $y) => new($x, $y as Alias)",
            "old($x, $y) => new($x, $y, __deprecate_argument)",
            "old($x, $y) => new(|| $x, $y)",
            "old($x, $y) => new!($x, $y)",
        ] {
            let recipe = Recipe::from_deprecation(&deprecation("old", template))
                .expect("valid template syntax")
                .expect("recipe exists");
            assert!(
                recipe
                    .render(&["api".into(), "old".into()], &["first()", "second()"])
                    .is_none(),
                "{template}"
            );
        }
    }
}
