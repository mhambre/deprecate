use crate::catalog;
use crate::helpers::version;
use crate::workspace::{Package, Workspace};
use deprecate::{Deprecation, Kind};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{braced, Attribute, Expr, ExprLit, Ident, Lit, LitStr, MetaNameValue, Token};

type Fields = BTreeMap<String, String>;

/// A discovered declaration with package and source context.
#[derive(Clone, Debug)]
pub struct Found {
    pub package_id: String,
    pub package: String,
    pub package_version: String,
    pub data: Deprecation,
    pub file: PathBuf,
    pub line: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SourceFile {
    file: PathBuf,
    module_dir: PathBuf,
    /// The directory the physical file resides in, used as the base for the
    /// first `#[path]` override found in the file (see `Scanner::override_base`).
    file_dir: PathBuf,
    scope: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SourceContext {
    pub package_id: String,
    pub crate_root: PathBuf,
    pub file: PathBuf,
    pub scope: Vec<String>,
}

/// Retains target and module context for each workspace source file.
pub fn source_contexts(workspace: &Workspace) -> Result<Vec<SourceContext>, String> {
    let mut sources = Vec::new();
    for package in workspace
        .packages
        .iter()
        .filter(|package| package.workspace)
    {
        walk_sources(package, &mut Vec::new(), &mut sources)?;
    }
    Ok(sources)
}

/// Discovers unique declarations from workspace packages and optional dependencies.
pub fn workspace(workspace: &Workspace, include_dependencies: bool) -> Result<Vec<Found>, String> {
    let mut found = Vec::new();
    for package in &workspace.packages {
        let has_catalog = package.root.join("deprecations.json").exists();
        if package.workspace || (include_dependencies && (package.uses_deprecate || has_catalog)) {
            scan_package(package, &mut found)?;
        }
    }
    sort_and_dedup(&mut found);
    Ok(found)
}

/// Orders findings and removes duplicate scans of the same package version.
fn sort_and_dedup(found: &mut Vec<Found>) {
    found.sort_by(|left, right| {
        (&left.package, &left.package_id, &left.data.name).cmp(&(
            &right.package,
            &right.package_id,
            &right.data.name,
        ))
    });
    found.dedup_by(|left, right| {
        left.package_id == right.package_id
            && left.data == right.data
            && left.file == right.file
            && left.line == right.line
    });
}

/// Reads a dependency catalog or parses current workspace source.
fn scan_package(package: &Package, found: &mut Vec<Found>) -> Result<(), String> {
    let catalog_path = package.root.join("deprecations.json");
    if !package.workspace && catalog_path.exists() {
        let catalog = catalog::read(&catalog_path)?;
        if catalog.package != package.name || catalog.version != package.version {
            return Err(format!(
                "catalog identity mismatch in {}: expected {}@{}, found {}@{}",
                catalog_path.display(),
                package.name,
                package.version,
                catalog.package,
                catalog.version
            ));
        }
        for data in catalog.deprecations {
            found.push(Found {
                package_id: package.id.clone(),
                package: package.name.clone(),
                package_version: package.version.clone(),
                data,
                file: catalog_path.clone(),
                line: 1,
                offset: 0,
            });
        }
        return Ok(());
    }
    walk_sources(package, found, &mut Vec::new())
}

/// Follows declared modules while rejecting recursive file inclusion.
fn walk_sources(
    package: &Package,
    found: &mut Vec<Found>,
    sources: &mut Vec<SourceContext>,
) -> Result<(), String> {
    let mut pending: Vec<_> = package
        .targets
        .iter()
        .map(|path| (target_source(path), path.clone(), Vec::<PathBuf>::new()))
        .collect();
    let mut visited = BTreeSet::new();
    while let Some((source, crate_root, mut ancestors)) = pending.pop() {
        let crate_root = crate_root
            .canonicalize()
            .map_err(|error| format!("failed to resolve {}: {error}", crate_root.display()))?;
        let file = source
            .file
            .canonicalize()
            .map_err(|error| format!("failed to resolve {}: {error}", source.file.display()))?;
        if ancestors.contains(&file) {
            return Err(format!("recursive module source: {}", file.display()));
        }
        ancestors.push(file.clone());
        if visited.insert((source.clone(), crate_root.clone())) {
            sources.push(SourceContext {
                package_id: package.id.clone(),
                crate_root: crate_root.clone(),
                file,
                scope: source.scope.clone(),
            });
            pending.extend(
                scan_file(package, &source, found)?
                    .into_iter()
                    .map(|child| (child, crate_root.clone(), ancestors.clone())),
            );
        }
    }
    Ok(())
}

/// Parses one Rust file and visits every item and macro declaration.
fn scan_file(
    package: &Package,
    source: &SourceFile,
    found: &mut Vec<Found>,
) -> Result<Vec<SourceFile>, String> {
    let input = fs::read_to_string(&source.file)
        .map_err(|error| format!("failed to read {}: {error}", source.file.display()))?;
    let syntax = syn::parse_file(&input)
        .map_err(|error| format!("failed to parse {}: {error}", source.file.display()))?;
    let mut scanner = Scanner {
        package,
        file: &source.file,
        found,
        errors: Vec::new(),
        scope: source.scope.clone(),
        module_dir: source.module_dir.clone(),
        override_base: Some(source.file_dir.clone()),
        external: Vec::new(),
        imports: macro_imports(&syntax.items),
    };
    scanner.visit_file(&syntax);
    if scanner.errors.is_empty() {
        Ok(scanner.external)
    } else {
        Err(scanner.errors.join("\n"))
    }
}

struct Scanner<'a> {
    package: &'a Package,
    file: &'a Path,
    found: &'a mut Vec<Found>,
    errors: Vec<String>,
    scope: Vec<String>,
    module_dir: PathBuf,
    /// The file directory available to the next `#[path]` override, consumed
    /// once any module (overridden or not) is entered one level deep.
    override_base: Option<PathBuf>,
    external: Vec<SourceFile>,
    imports: BTreeMap<String, Vec<String>>,
}

impl Scanner<'_> {
    /// Records all `deprecate::item` attributes attached to one named item.
    fn attributes(&mut self, attributes: &[Attribute], name: &str) {
        let name = self.qualified_name(name);
        for attribute in attributes {
            if !macro_path(attribute.path(), "item", &self.imports) {
                continue;
            }
            match attribute_fields(attribute)
                .and_then(|fields| deprecation(name.clone(), Kind::Item, &fields))
            {
                Ok(data) => self.push(
                    data,
                    attribute.span().start().line,
                    attribute.span().byte_range().start,
                ),
                Err(error) => self
                    .errors
                    .push(format!("{}: {error}", self.file.display())),
            }
        }
    }

    /// Records every declaration in one `deprecate::features!` invocation.
    fn features(&mut self, declaration: &syn::Macro) {
        if !macro_path(&declaration.path, "features", &self.imports) {
            return;
        }
        match syn::parse2::<FeatureDeclarations>(declaration.tokens.clone()) {
            Ok(declarations) => {
                for declaration in declarations.entries {
                    match deprecation(declaration.name, Kind::Feature, &declaration.fields) {
                        Ok(data) => self.push(data, declaration.line, 0),
                        Err(error) => self.errors.push(format!(
                            "{}:{}: {error}",
                            self.file.display(),
                            declaration.line
                        )),
                    }
                }
            }
            Err(error) => self
                .errors
                .push(format!("{}: {error}", self.file.display())),
        }
    }

    /// Adds package and source context to parsed lifecycle data.
    fn push(&mut self, data: Deprecation, line: usize, offset: usize) {
        self.found.push(Found {
            package_id: self.package.id.clone(),
            package: self.package.name.clone(),
            package_version: self.package.version.clone(),
            data,
            file: self.file.to_path_buf(),
            line,
            offset,
        });
    }

    /// Joins an item name to its source module or owning type.
    fn qualified_name(&self, name: &str) -> String {
        self.scope
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(name))
            .collect::<Vec<_>>()
            .join("::")
    }
}

impl<'ast> Visit<'ast> for Scanner<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if let Some((attributes, name)) = item_parts(item) {
            self.attributes(attributes, &name.to_string());
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        if let Some((attributes, name)) = impl_item_parts(item) {
            self.attributes(attributes, &name.to_string());
        }
        syn::visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        if let Some((attributes, name)) = trait_item_parts(item) {
            self.attributes(attributes, &name.to_string());
        }
        syn::visit::visit_trait_item(self, item);
    }

    fn visit_macro(&mut self, declaration: &'ast syn::Macro) {
        self.features(declaration);
        syn::visit::visit_macro(self, declaration);
    }

    /// Applies inline path overrides before locating descendant module files.
    ///
    /// A `#[path]` override on the first module reached from a file is
    /// relative to that file's own directory, not the conventional
    /// subdirectory named after the file. Once any module (overridden or
    /// not) has been entered, that escape hatch closes: further overrides
    /// compose onto the current module directory like rustc does.
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if let Some((_, items)) = &item.content {
            let imports = std::mem::replace(&mut self.imports, macro_imports(items));
            self.scope.push(item.ident.to_string());
            let previous_dir = self.module_dir.clone();
            let previous_base = self.override_base.take();
            self.module_dir = match item.attrs.iter().find_map(path_attribute) {
                Some(path) => previous_base.as_deref().unwrap_or(&previous_dir).join(path),
                None => previous_dir.join(item.ident.to_string()),
            };
            syn::visit::visit_item_mod(self, item);
            self.module_dir = previous_dir;
            self.override_base = previous_base;
            self.scope.pop();
            self.imports = imports;
        } else if let Some(source) = external_module(
            &self.module_dir,
            self.override_base.as_deref(),
            &self.scope,
            item,
        ) {
            self.external.push(source);
        }
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let mut imports = self.imports.clone();
        for statement in &block.stmts {
            if let syn::Stmt::Item(syn::Item::Use(item)) = statement {
                collect_imports(&item.tree, &[], &mut imports);
            }
        }
        let previous = std::mem::replace(&mut self.imports, imports);
        syn::visit::visit_block(self, block);
        self.imports = previous;
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.scope.push(item.ident.to_string());
        syn::visit::visit_item_trait(self, item);
        self.scope.pop();
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let added = type_path(&item.self_ty).map(|path| {
            let count = path.len();
            self.scope.extend(path);
            count
        });
        syn::visit::visit_item_impl(self, item);
        if let Some(count) = added {
            self.scope.truncate(self.scope.len() - count);
        }
    }
}

/// Creates a root module from a Cargo target entry point.
fn target_source(path: &Path) -> SourceFile {
    let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
    SourceFile {
        file: path.to_path_buf(),
        module_dir: dir.clone(),
        file_dir: dir,
        scope: Vec::new(),
    }
}

/// Resolves an external module using Rust's conventional file locations.
fn external_module(
    module_dir: &Path,
    override_base: Option<&Path>,
    scope: &[String],
    item: &syn::ItemMod,
) -> Option<SourceFile> {
    let name = item.ident.to_string();
    let explicit = item.attrs.iter().find_map(path_attribute);
    let file = explicit.map_or_else(
        || {
            let file = module_dir.join(format!("{name}.rs"));
            if file.exists() {
                file
            } else {
                module_dir.join(&name).join("mod.rs")
            }
        },
        |path| override_base.unwrap_or(module_dir).join(path),
    );
    file.exists().then(|| SourceFile {
        module_dir: child_module_dir(&file),
        file_dir: file.parent().unwrap_or(Path::new("")).to_path_buf(),
        file,
        scope: scope.iter().cloned().chain(std::iter::once(name)).collect(),
    })
}

/// Derives where the resolved module file searches for child modules.
fn child_module_dir(file: &Path) -> PathBuf {
    if file.file_name().is_some_and(|name| name == "mod.rs") {
        file.parent().unwrap_or(Path::new("")).to_path_buf()
    } else {
        file.with_extension("")
    }
}

/// Reads a string-valued `path` module attribute.
fn path_attribute(attribute: &Attribute) -> Option<PathBuf> {
    if !attribute.path().is_ident("path") {
        return None;
    }
    let syn::Meta::NameValue(value) = &attribute.meta else {
        return None;
    };
    let Expr::Lit(ExprLit {
        lit: Lit::Str(path),
        ..
    }) = &value.value
    else {
        return None;
    };
    Some(PathBuf::from(path.value()))
}

/// Extracts a simple owning type path for implementation items.
fn type_path(ty: &syn::Type) -> Option<Vec<String>> {
    let syn::Type::Path(ty) = ty else {
        return None;
    };
    Some(vec![ty.path.segments.last()?.ident.to_string()])
}

/// Returns attributes and a name from a supported top-level item.
fn item_parts(item: &syn::Item) -> Option<(&[Attribute], &Ident)> {
    match item {
        syn::Item::Const(item) => Some((&item.attrs, &item.ident)),
        syn::Item::Enum(item) => Some((&item.attrs, &item.ident)),
        syn::Item::Fn(item) => Some((&item.attrs, &item.sig.ident)),
        syn::Item::Macro(item) => Some((&item.attrs, item.ident.as_ref()?)),
        syn::Item::Mod(item) => Some((&item.attrs, &item.ident)),
        syn::Item::Static(item) => Some((&item.attrs, &item.ident)),
        syn::Item::Struct(item) => Some((&item.attrs, &item.ident)),
        syn::Item::Trait(item) => Some((&item.attrs, &item.ident)),
        syn::Item::TraitAlias(item) => Some((&item.attrs, &item.ident)),
        syn::Item::Type(item) => Some((&item.attrs, &item.ident)),
        syn::Item::Union(item) => Some((&item.attrs, &item.ident)),
        _ => None,
    }
}

/// Returns attributes and a name from a supported implementation item.
fn impl_item_parts(item: &syn::ImplItem) -> Option<(&[Attribute], &Ident)> {
    match item {
        syn::ImplItem::Const(item) => Some((&item.attrs, &item.ident)),
        syn::ImplItem::Fn(item) => Some((&item.attrs, &item.sig.ident)),
        syn::ImplItem::Type(item) => Some((&item.attrs, &item.ident)),
        _ => None,
    }
}

/// Returns attributes and a name from a supported trait item.
fn trait_item_parts(item: &syn::TraitItem) -> Option<(&[Attribute], &Ident)> {
    match item {
        syn::TraitItem::Const(item) => Some((&item.attrs, &item.ident)),
        syn::TraitItem::Fn(item) => Some((&item.attrs, &item.sig.ident)),
        syn::TraitItem::Type(item) => Some((&item.attrs, &item.ident)),
        _ => None,
    }
}

/// Resolves local macro imports before recognizing an annotation.
fn macro_path(path: &syn::Path, name: &str, imports: &BTreeMap<String, Vec<String>>) -> bool {
    let mut segments: Vec<_> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let mut visited = BTreeSet::new();
    while let Some(first) = segments.first() {
        if !visited.insert(first.clone()) {
            return false;
        }
        let Some(import) = imports.get(first) else {
            break;
        };
        if import.as_slice() == [first.as_str()] {
            break;
        }
        segments = import
            .iter()
            .cloned()
            .chain(segments.into_iter().skip(1))
            .collect();
    }
    segments == ["deprecate", name]
}

fn macro_imports(items: &[syn::Item]) -> BTreeMap<String, Vec<String>> {
    let mut imports = BTreeMap::new();
    for item in items {
        if let syn::Item::Use(item) = item {
            collect_imports(&item.tree, &[], &mut imports);
        }
    }
    imports
}

fn collect_imports(
    tree: &syn::UseTree,
    prefix: &[String],
    imports: &mut BTreeMap<String, Vec<String>>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut prefix = prefix.to_vec();
            prefix.push(path.ident.to_string());
            collect_imports(&path.tree, &prefix, imports);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_imports(item, prefix, imports);
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
            imports.insert(name, path);
        }
        syn::UseTree::Rename(rename) => {
            let mut path = prefix.to_vec();
            if rename.ident != "self" {
                path.push(rename.ident.to_string());
            }
            imports.insert(rename.rename.to_string(), path);
        }
        syn::UseTree::Glob(_) => {
            if prefix == ["deprecate"] {
                for name in ["item", "features"] {
                    imports
                        .entry(name.to_owned())
                        .or_insert_with(|| vec!["deprecate".to_owned(), name.to_owned()]);
                }
            }
        }
    }
}

/// Parses string-valued item attribute fields.
fn attribute_fields(attribute: &Attribute) -> syn::Result<Fields> {
    let values =
        attribute.parse_args_with(Punctuated::<MetaNameValue, Token![,]>::parse_terminated)?;
    fields_from_meta(values)
}

/// Converts syntax-level name-value pairs into owned metadata.
fn fields_from_meta(values: Punctuated<MetaNameValue, Token![,]>) -> syn::Result<Fields> {
    let mut fields = Fields::new();
    for value in values {
        let key = value
            .path
            .get_ident()
            .ok_or_else(|| syn::Error::new(value.path.span(), "expected a field name"))?
            .to_string();
        if !matches!(
            key.as_str(),
            "since" | "remove" | "replacement" | "reason" | "migrate"
        ) {
            return Err(syn::Error::new(
                value.path.span(),
                format!("unknown deprecation field `{key}`"),
            ));
        }
        let Expr::Lit(ExprLit {
            lit: Lit::Str(literal),
            ..
        }) = value.value
        else {
            return Err(syn::Error::new(
                value.value.span(),
                "expected a string literal",
            ));
        };
        if fields.insert(key, literal.value()).is_some() {
            return Err(syn::Error::new(
                literal.span(),
                "duplicate deprecation field",
            ));
        }
    }
    Ok(fields)
}

/// Converts parsed fields into the shared catalog model.
fn deprecation(name: String, kind: Kind, fields: &Fields) -> syn::Result<Deprecation> {
    let since = fields.get("since").ok_or_else(|| {
        syn::Error::new(proc_macro2::Span::call_site(), "missing required `since`")
    })?;
    let data = Deprecation {
        name,
        kind,
        since: since.clone(),
        remove: fields.get("remove").cloned(),
        replacement: fields.get("replacement").cloned(),
        reason: fields.get("reason").cloned(),
        migrate: fields.get("migrate").cloned(),
    };
    version::validate(&data.since, data.remove.as_deref())
        .map_err(|error| syn::Error::new(proc_macro2::Span::call_site(), error))?;
    Ok(data)
}

struct FeatureDeclarations {
    entries: Vec<FeatureDeclaration>,
}

impl Parse for FeatureDeclarations {
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
    line: usize,
}

impl Parse for FeatureDeclaration {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse::<LitStr>()?;
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
            let literal = content.parse::<LitStr>()?;
            if fields.insert(key.to_string(), literal.value()).is_some() {
                return Err(syn::Error::new(key.span(), "duplicate deprecation field"));
            }
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }
        Ok(Self {
            line: name.span().start().line,
            name: name.value(),
            fields,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{attribute_fields, sort_and_dedup, target_source, FeatureDeclarations, Found};
    use deprecate::{Deprecation, Kind};
    use std::path::{Path, PathBuf};
    use syn::parse::Parser;

    #[test]
    fn parses_item_fields() {
        let attribute: syn::Attribute = syn::Attribute::parse_outer
            .parse_str(r#"#[deprecate::item(since = "1.2", remove = "2", replacement = "new")]"#)
            .expect("attribute parses")
            .remove(0);
        let fields = attribute_fields(&attribute).expect("fields parse");
        assert_eq!(fields["since"], "1.2");
        assert_eq!(fields.len(), 3);
    }

    #[test]
    fn parses_feature_entries() {
        let entries = syn::parse_str::<FeatureDeclarations>(
            r#""old" => { since: "1", replacement: "new" },"#,
        )
        .expect("features parse");
        assert_eq!(entries.entries[0].name, "old");
        assert_eq!(entries.entries[0].fields.len(), 2);
    }

    #[test]
    fn treats_custom_target_path_as_crate_root() {
        let source = target_source(Path::new("custom/lib.rs"));
        assert!(source.scope.is_empty());
        assert_eq!(source.module_dir, Path::new("custom"));
    }

    #[test]
    fn retains_same_declaration_from_multiple_package_versions() {
        let entry = |id: &str, version: &str| Found {
            package_id: id.to_owned(),
            package: "dependency".to_owned(),
            package_version: version.to_owned(),
            data: Deprecation {
                name: "old".to_owned(),
                kind: Kind::Item,
                since: "1".to_owned(),
                remove: None,
                replacement: None,
                reason: None,
                migrate: None,
            },
            file: PathBuf::from("lib.rs"),
            line: 1,
            offset: 0,
        };
        let mut found = vec![
            entry("dependency@1", "1.0.0"),
            entry("dependency@2", "2.0.0"),
        ];
        sort_and_dedup(&mut found);
        assert_eq!(found.len(), 2);
    }
}
