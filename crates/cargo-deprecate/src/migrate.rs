use crate::helpers::recipe::Recipe;
use crate::resolve::{Resolver, Source};
use crate::scan::Found;
use crate::workspace::DiagnosticSpan;
use deprecate::Kind;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::fs;
use std::ops::Range;
use std::path::PathBuf;
use syn::spanned::Spanned;
use syn::visit::Visit;

#[derive(Debug)]
pub struct FileChange {
    pub path: PathBuf,
    pub edits: Vec<Edit>,
    pub input: String,
    pub output: String,
}

#[derive(Debug)]
pub struct Edit {
    pub line: usize,
    pub before: String,
    pub after: String,
}

struct Declaration<'a> {
    found: &'a Found,
    recipe: Recipe,
}

/// Keeps each recipe tied to its declaring package.
fn declarations(entries: &[Found]) -> Result<Vec<Declaration<'_>>, String> {
    entries
        .iter()
        .filter(|entry| entry.data.kind == Kind::Item)
        .map(|found| {
            Recipe::from_deprecation(&found.data)
                .map(|recipe| recipe.map(|recipe| Declaration { found, recipe }))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|entries| entries.into_iter().flatten().collect())
}

/// Plans nonoverlapping edits with unambiguous ownership and compiler spans.
pub fn plan(
    resolver: &Resolver<'_>,
    entries: &[Found],
    spans: &[DiagnosticSpan],
) -> Result<(Vec<FileChange>, usize), String> {
    let recipes = declarations(entries)?;
    let mut changes = Vec::new();
    let mut handled = BTreeSet::new();
    let mut automated = BTreeSet::new();
    for source in &resolver.sources {
        if resolver
            .sources
            .iter()
            .filter(|other| other.context.file == source.context.file)
            .count()
            != 1
        {
            continue;
        }
        let file_spans: Vec<_> = spans
            .iter()
            .filter(|span| {
                span.file == source.context.file
                    && span.package_id == source.context.package_id
                    && span.crate_root == source.context.crate_root
            })
            .collect();
        if file_spans.is_empty() || !handled.insert(source.context.file.clone()) {
            continue;
        }
        let mut replacements = find_calls(source, resolver, &recipes, Some(&file_spans));
        replacements.sort_by_key(|change| change.range.start);
        let mut output = source.input.clone();
        let mut filtered: Vec<Replacement> = Vec::new();
        for candidate in replacements {
            if !filtered
                .last()
                .is_some_and(|previous| previous.range.end > candidate.range.start)
            {
                filtered.push(candidate);
            }
        }
        let mut edits = Vec::new();
        for replacement in filtered.into_iter().rev() {
            automated.insert((
                source.context.file.clone(),
                replacement.identifier.start,
                replacement.identifier.end,
            ));
            edits.push(Edit {
                line: source.input[..replacement.range.start]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1,
                before: source.input[replacement.range.clone()].to_owned(),
                after: replacement.text.clone(),
            });
            output.replace_range(replacement.range, &replacement.text);
        }
        if !edits.is_empty() {
            syn::parse_file(&output).map_err(|error| {
                format!(
                    "invalid migration output for {}: {error}",
                    source.context.file.display()
                )
            })?;
            edits.reverse();
            changes.push(FileChange {
                path: source.context.file.clone(),
                edits,
                input: source.input.clone(),
                output,
            });
        }
    }
    let manual = spans
        .iter()
        .filter(|span| {
            !automated.iter().any(|(file, start, end)| {
                file == &span.file && span.start <= *start && span.end >= *end
            })
        })
        .map(|span| (&span.file, span.start, span.end))
        .collect::<BTreeSet<_>>()
        .len();
    Ok((changes, manual))
}

/// Counts syntactic candidates without claiming compiler confirmation.
pub fn count_usages(resolver: &Resolver<'_>, entry: &Found) -> Result<usize, String> {
    let recipes = declarations(std::slice::from_ref(entry))?;
    Ok(resolver
        .sources
        .iter()
        .map(|source| find_calls(source, resolver, &recipes, None).len())
        .sum())
}

/// Checks source snapshots, writes edits, and restores them if compilation fails.
pub fn apply(
    changes: &[FileChange],
    workspace: &crate::workspace::Workspace,
) -> Result<(), String> {
    for change in changes {
        let current = fs::read_to_string(&change.path)
            .map_err(|error| format!("failed to read {}: {error}", change.path.display()))?;
        if current != change.input {
            return Err(format!(
                "source changed during migration: {}",
                change.path.display()
            ));
        }
    }
    for (index, change) in changes.iter().enumerate() {
        if let Err(error) = fs::write(&change.path, &change.output) {
            let mut message = format!("failed to write {}: {error}", change.path.display());
            for previous in &changes[..=index] {
                if let Err(error) = fs::write(&previous.path, &previous.input) {
                    let _ = write!(
                        message,
                        "\nfailed to restore {}: {error}",
                        previous.path.display()
                    );
                }
            }
            return Err(message);
        }
    }
    if let Err(error) = crate::workspace::deprecated_spans(workspace) {
        let mut message = format!("migration did not compile; restoring source files:\n{error}");
        for change in changes {
            match fs::read_to_string(&change.path) {
                Ok(current) if current == change.output => {
                    if let Err(error) = fs::write(&change.path, &change.input) {
                        let _ = write!(
                            message,
                            "\nfailed to restore {}: {error}",
                            change.path.display()
                        );
                    }
                }
                _ => {
                    let _ = write!(
                        message,
                        "\nsource changed during validation; restore skipped for {}",
                        change.path.display()
                    );
                }
            }
        }
        return Err(message);
    }
    Ok(())
}

struct Replacement {
    range: Range<usize>,
    identifier: Range<usize>,
    text: String,
}

/// Visits parsed calls without rewriting comments, literals, or macro tokens.
fn find_calls(
    source: &Source,
    resolver: &Resolver<'_>,
    recipes: &[Declaration<'_>],
    spans: Option<&[&DiagnosticSpan]>,
) -> Vec<Replacement> {
    let mut finder = CallFinder {
        source,
        resolver,
        recipes,
        spans,
        replacements: Vec::new(),
        scope: source.context.scope.clone(),
        blocked: false,
    };
    finder.visit_file(&source.syntax);
    finder.replacements
}

struct CallFinder<'a, 'workspace> {
    source: &'a Source,
    resolver: &'a Resolver<'workspace>,
    recipes: &'a [Declaration<'a>],
    spans: Option<&'a [&'a DiagnosticSpan]>,
    replacements: Vec<Replacement>,
    scope: Vec<String>,
    blocked: bool,
}

impl Visit<'_> for CallFinder<'_, '_> {
    /// Tracks inline modules independently of the physical source file.
    fn visit_item_mod(&mut self, item: &syn::ItemMod) {
        self.scope.push(item.ident.to_string());
        syn::visit::visit_item_mod(self, item);
        self.scope.pop();
    }

    /// Leaves scopes with local item or macro bindings to the compiler.
    fn visit_block(&mut self, block: &syn::Block) {
        let previous = self.blocked;
        self.blocked |= block
            .stmts
            .iter()
            .any(|statement| matches!(statement, syn::Stmt::Item(_) | syn::Stmt::Macro(_)));
        syn::visit::visit_block(self, block);
        self.blocked = previous;
    }

    /// Type parameters can shadow module paths in the type namespace.
    fn visit_item_fn(&mut self, item: &syn::ItemFn) {
        let previous = self.blocked;
        self.blocked |= !item.sig.generics.params.is_empty();
        syn::visit::visit_item_fn(self, item);
        self.blocked = previous;
    }

    /// Associated scopes require type resolution, which this resolver omits.
    fn visit_item_impl(&mut self, _item: &syn::ItemImpl) {}

    /// Trait defaults have the same unresolved associated scope as impls.
    fn visit_item_trait(&mut self, _item: &syn::ItemTrait) {}

    fn visit_expr_call(&mut self, call: &syn::ExprCall) {
        if let Some(replacement) = self.replacement(call) {
            self.replacements.push(replacement);
        }
        syn::visit::visit_expr_call(self, call);
    }
}

impl CallFinder<'_, '_> {
    /// Requires one declaration and leaves unsupported call syntax untouched.
    fn replacement(&self, call: &syn::ExprCall) -> Option<Replacement> {
        let syn::Expr::Path(function) = call.func.as_ref() else {
            return None;
        };
        if self.blocked
            || function.qself.is_some()
            || function.path.segments.len() < 2
            || function
                .path
                .segments
                .iter()
                .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
        {
            return None;
        }
        let identifier = function.path.segments.last()?.ident.span().byte_range();
        if self.spans.is_some_and(|spans| {
            !spans
                .iter()
                .any(|span| span.start <= identifier.start && span.end >= identifier.end)
        }) {
            return None;
        }
        let identity = self
            .resolver
            .resolve(&self.source.context, &self.scope, &function.path)?;
        let matching: Vec<_> = self
            .recipes
            .iter()
            .filter(|entry| {
                entry.found.package_id == identity.package_id
                    && entry.found.data.name == identity.name
                    && identity
                        .declaration
                        .as_ref()
                        .map_or(true, |(file, attributes)| {
                            entry.found.file.canonicalize().ok().as_ref() == Some(file)
                                && attributes.contains(&entry.found.offset)
                        })
            })
            .collect();
        let [declaration] = matching.as_slice() else {
            return None;
        };
        let arguments = call
            .args
            .iter()
            .map(|argument| &self.source.input[argument.span().byte_range()])
            .collect::<Vec<_>>();
        let mut path = function
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if function.path.leading_colon.is_some() {
            path.insert(0, String::new());
        }
        let text = declaration.recipe.render(&path, &arguments)?;
        let range = call.span().byte_range();
        if text == self.source.input[range.clone()] {
            return None;
        }
        Some(Replacement {
            range,
            identifier,
            text,
        })
    }
}
