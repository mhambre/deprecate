use crate::scan::{self, SourceContext};
use crate::workspace::Workspace;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use syn::spanned::Spanned;

pub struct Source {
    pub context: SourceContext,
    pub input: String,
    pub syntax: syn::File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity {
    pub package_id: String,
    pub name: String,
    pub declaration: Option<(PathBuf, Vec<usize>)>,
}

enum Binding {
    Module,
    Function {
        file: PathBuf,
        attributes: Vec<usize>,
    },
    Import {
        path: Vec<String>,
        absolute: bool,
    },
    Unknown,
}

#[derive(Default)]
struct Module {
    bindings: BTreeMap<String, Binding>,
    opaque: bool,
}

pub struct Resolver<'a> {
    pub sources: Vec<Source>,
    modules: BTreeMap<(PathBuf, Vec<String>), Module>,
    workspace: &'a Workspace,
}

impl<'a> Resolver<'a> {
    /// Indexes only Cargo crate roots and their declared module files.
    pub fn load(workspace: &'a Workspace) -> Result<Self, String> {
        let boundary = workspace
            .root
            .canonicalize()
            .map_err(|error| format!("failed to resolve workspace root: {error}"))?;
        let mut resolver = Self {
            sources: Vec::new(),
            modules: BTreeMap::new(),
            workspace,
        };
        for context in scan::source_contexts(workspace)? {
            if !context.file.starts_with(&boundary) {
                continue;
            }
            let input = fs::read_to_string(&context.file)
                .map_err(|error| format!("failed to read {}: {error}", context.file.display()))?;
            let syntax = syn::parse_file(&input)
                .map_err(|error| format!("failed to parse {}: {error}", context.file.display()))?;
            resolver.index(&context, &context.scope, &syntax.items);
            resolver.sources.push(Source {
                context,
                input,
                syntax,
            });
        }
        Ok(resolver)
    }

    /// Marks conditional or duplicate bindings as unknown instead of guessing.
    fn index(&mut self, context: &SourceContext, scope: &[String], items: &[syn::Item]) {
        let mut module = Module::default();
        for item in items {
            match item {
                syn::Item::Mod(item) => {
                    insert(
                        &mut module,
                        item.ident.to_string(),
                        if conditional(&item.attrs) {
                            Binding::Unknown
                        } else {
                            Binding::Module
                        },
                    );
                    if let Some((_, items)) = &item.content {
                        let mut child = scope.to_vec();
                        child.push(item.ident.to_string());
                        self.index(context, &child, items);
                    }
                }
                syn::Item::Fn(item) => insert(
                    &mut module,
                    item.sig.ident.to_string(),
                    if conditional(&item.attrs) {
                        Binding::Unknown
                    } else {
                        Binding::Function {
                            file: context.file.clone(),
                            attributes: item
                                .attrs
                                .iter()
                                .map(|attr| attr.span().byte_range().start)
                                .collect(),
                        }
                    },
                ),
                syn::Item::Use(item) if !conditional(&item.attrs) => {
                    imports(&item.tree, &[], item.leading_colon.is_some(), &mut module);
                }
                syn::Item::Const(item) => {
                    insert(&mut module, item.ident.to_string(), Binding::Unknown);
                }
                syn::Item::Static(item) => {
                    insert(&mut module, item.ident.to_string(), Binding::Unknown);
                }
                syn::Item::Struct(item) => {
                    insert(&mut module, item.ident.to_string(), Binding::Unknown);
                }
                syn::Item::Enum(item) => {
                    insert(&mut module, item.ident.to_string(), Binding::Unknown);
                }
                syn::Item::Type(item) => {
                    insert(&mut module, item.ident.to_string(), Binding::Unknown);
                }
                syn::Item::Trait(item) => {
                    insert(&mut module, item.ident.to_string(), Binding::Unknown);
                }
                syn::Item::Union(item) => {
                    insert(&mut module, item.ident.to_string(), Binding::Unknown);
                }
                syn::Item::Impl(_) => {}
                _ => module.opaque = true,
            }
        }
        let key = (context.crate_root.clone(), scope.to_vec());
        if self.modules.contains_key(&key) {
            module
                .bindings
                .values_mut()
                .for_each(|binding| *binding = Binding::Unknown);
            module.opaque = true;
        }
        self.modules.insert(key, module);
    }

    /// Resolves supported paths to a package ID and declaration path.
    pub fn resolve(
        &self,
        context: &SourceContext,
        scope: &[String],
        path: &syn::Path,
    ) -> Option<Identity> {
        if self
            .workspace
            .packages
            .iter()
            .any(|package| package.id == context.package_id && package.edition == "2015")
        {
            return None;
        }
        let segments = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        self.path(
            context,
            scope,
            &segments,
            path.leading_colon.is_some(),
            &mut BTreeSet::new(),
        )
    }

    /// Follows aliases with cycle and depth guards.
    fn path(
        &self,
        context: &SourceContext,
        scope: &[String],
        segments: &[String],
        absolute: bool,
        visited: &mut BTreeSet<(Vec<String>, Vec<String>, bool)>,
    ) -> Option<Identity> {
        if segments.is_empty()
            || visited.len() >= 64
            || !visited.insert((scope.to_vec(), segments.to_vec(), absolute))
        {
            return None;
        }
        if absolute {
            return self.external(context, segments);
        }
        match segments[0].as_str() {
            "crate" => return self.local(context, &[], &segments[1..], visited),
            "self" => return self.local(context, scope, &segments[1..], visited),
            "super" => {
                return self.path(
                    context,
                    scope.get(..scope.len().checked_sub(1)?)?,
                    &segments[1..],
                    false,
                    visited,
                )
            }
            _ => {}
        }
        let module = self
            .modules
            .get(&(context.crate_root.clone(), scope.to_vec()))?;
        if module.bindings.contains_key(&segments[0]) {
            self.local(context, scope, segments, visited)
        } else if !module.opaque {
            self.external(context, segments)
        } else {
            None
        }
    }

    /// Resolves explicit module bindings, excluding function reexports.
    fn local(
        &self,
        context: &SourceContext,
        scope: &[String],
        segments: &[String],
        visited: &mut BTreeSet<(Vec<String>, Vec<String>, bool)>,
    ) -> Option<Identity> {
        let first = segments.first()?;
        let module = self
            .modules
            .get(&(context.crate_root.clone(), scope.to_vec()))?;
        match module.bindings.get(first)? {
            Binding::Module => {
                let mut child = scope.to_vec();
                child.push(first.clone());
                self.local(context, &child, &segments[1..], visited)
            }
            Binding::Function { file, attributes } if segments.len() == 1 => Some(Identity {
                package_id: context.package_id.clone(),
                declaration: Some((file.clone(), attributes.clone())),
                name: scope
                    .iter()
                    .chain(segments)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("::"),
            }),
            Binding::Import { path, absolute } => {
                if segments.len() == 1 {
                    return None;
                }
                let path = path
                    .iter()
                    .chain(&segments[1..])
                    .cloned()
                    .collect::<Vec<_>>();
                self.path(context, scope, &path, *absolute, visited)
            }
            _ => None,
        }
    }

    /// Uses Cargo's dependency aliases rather than diagnostic display names.
    fn external(&self, context: &SourceContext, segments: &[String]) -> Option<Identity> {
        if segments.len() < 2 {
            return None;
        }
        let package_id = self
            .workspace
            .dependencies
            .get(&context.package_id)?
            .get(&segments[0])?;
        Some(Identity {
            package_id: package_id.clone(),
            declaration: None,
            name: segments[1..].join("::"),
        })
    }
}

/// Detects bindings that require configuration evaluation.
fn conditional(attributes: &[syn::Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr"))
}

/// Makes duplicate names ineligible for automatic resolution.
fn insert(module: &mut Module, name: String, binding: Binding) {
    match module.bindings.entry(name) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(binding);
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            entry.insert(Binding::Unknown);
        }
    }
}

/// Flattens grouped imports while retaining their source paths.
fn imports(tree: &syn::UseTree, prefix: &[String], absolute: bool, module: &mut Module) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut prefix = prefix.to_vec();
            prefix.push(path.ident.to_string());
            imports(&path.tree, &prefix, absolute, module);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                imports(item, prefix, absolute, module);
            }
        }
        syn::UseTree::Name(name) => {
            let mut path = prefix.to_vec();
            let name = if name.ident == "self" {
                let Some(name) = prefix.last() else {
                    return;
                };
                name.clone()
            } else {
                path.push(name.ident.to_string());
                name.ident.to_string()
            };
            insert(module, name, Binding::Import { path, absolute });
        }
        syn::UseTree::Rename(rename) => {
            let mut path = prefix.to_vec();
            if rename.ident != "self" {
                path.push(rename.ident.to_string());
            }
            insert(
                module,
                rename.rename.to_string(),
                Binding::Import { path, absolute },
            );
        }
        syn::UseTree::Glob(_) => module.opaque = true,
    }
}
