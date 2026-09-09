use cargo_metadata::{Message, MetadataCommand, TargetKind};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

/// Cargo package data needed by discovery and lifecycle checks.
#[derive(Clone, Debug)]
pub struct Package {
    pub id: String,
    pub name: String,
    pub version: String,
    pub edition: String,
    pub root: PathBuf,
    pub targets: Vec<PathBuf>,
    pub workspace: bool,
    pub declared_features: BTreeSet<String>,
    pub uses_deprecate: bool,
}

/// The resolved package graph and its workspace boundary.
#[derive(Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub packages: Vec<Package>,
    pub enabled_features: BTreeMap<String, BTreeSet<String>>,
    pub dependencies: BTreeMap<String, BTreeMap<String, String>>,
}

/// A source range rustc resolved as a deprecated use.
#[derive(Clone, Debug)]
pub struct DiagnosticSpan {
    pub crate_root: PathBuf,
    pub package_id: String,
    pub file: PathBuf,
    pub start: usize,
    pub end: usize,
}

/// Loads the complete resolved graph through `cargo metadata`.
pub fn load() -> Result<Workspace, String> {
    let metadata = MetadataCommand::new()
        .exec()
        .map_err(|error| format!("failed to load Cargo metadata: {error}"))?;
    let members: BTreeSet<_> = metadata
        .workspace_members
        .iter()
        .map(ToString::to_string)
        .collect();
    let packages = metadata
        .packages
        .into_iter()
        .map(|package| {
            let id = package.id.to_string();
            let root = package
                .manifest_path
                .parent()
                .expect("Cargo manifest paths have a parent")
                .as_std_path()
                .to_path_buf();
            Package {
                workspace: members.contains(&id),
                id,
                root,
                targets: package
                    .targets
                    .iter()
                    .filter(|target| source_target(&target.kind))
                    .map(|target| target.src_path.clone().into_std_path_buf())
                    .collect(),
                name: package.name,
                version: package.version.to_string(),
                edition: package.edition.to_string(),
                declared_features: package.features.into_keys().collect(),
                uses_deprecate: package
                    .dependencies
                    .iter()
                    .any(|dependency| dependency.name == "deprecate"),
            }
        })
        .collect();
    let dependencies = metadata
        .resolve
        .as_ref()
        .map(|resolve| {
            resolve
                .nodes
                .iter()
                .map(|node| (node.id.to_string(), dependency_aliases(&node.deps)))
                .collect()
        })
        .unwrap_or_default();
    let enabled_features = metadata
        .resolve
        .map(|resolve| {
            resolve
                .nodes
                .into_iter()
                .map(|node| (node.id.to_string(), node.features.into_iter().collect()))
                .collect()
        })
        .unwrap_or_default();
    Ok(Workspace {
        root: metadata.workspace_root.into_std_path_buf(),
        packages,
        enabled_features,
        dependencies,
    })
}

/// Omits aliases that select different packages across dependency contexts.
fn dependency_aliases(dependencies: &[cargo_metadata::NodeDep]) -> BTreeMap<String, String> {
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for dependency in dependencies {
        candidates
            .entry(dependency.name.clone())
            .or_default()
            .insert(dependency.pkg.to_string());
    }
    candidates
        .into_iter()
        .filter_map(|(name, packages)| {
            if packages.len() == 1 {
                packages.into_iter().next().map(|package| (name, package))
            } else {
                None
            }
        })
        .collect()
}

/// Selects crate targets that can contain shipped API declarations.
fn source_target(kinds: &[TargetKind]) -> bool {
    !kinds.iter().any(|kind| {
        matches!(
            kind,
            TargetKind::Bench | TargetKind::CustomBuild | TargetKind::Example | TargetKind::Test
        )
    })
}

/// Collects primary deprecated-use spans from Cargo's JSON diagnostics.
pub fn deprecated_spans(workspace: &Workspace) -> Result<Vec<DiagnosticSpan>, String> {
    let output = Command::new("cargo")
        .args(["check", "--workspace", "--message-format=json"])
        .current_dir(&workspace.root)
        .output()
        .map_err(|error| format!("failed to run cargo check: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo check must succeed before migrations can be applied:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut spans = Vec::new();
    for message in Message::parse_stream(Cursor::new(output.stdout)) {
        let Message::CompilerMessage(message) =
            message.map_err(|error| format!("failed to parse Cargo diagnostics: {error}"))?
        else {
            continue;
        };
        if message.message.code.as_ref().map(|code| code.code.as_str()) != Some("deprecated") {
            continue;
        }
        let crate_root = message.target.src_path.into_std_path_buf();
        let crate_root = crate_root
            .canonicalize()
            .map_err(|error| format!("failed to resolve {}: {error}", crate_root.display()))?;
        for span in message.message.spans {
            if !span.is_primary || span.expansion.is_some() {
                continue;
            }
            let file = PathBuf::from(span.file_name);
            let file = if file.is_absolute() {
                file
            } else {
                workspace.root.join(file)
            };
            let file = file
                .canonicalize()
                .map_err(|error| format!("failed to resolve {}: {error}", file.display()))?;
            spans.push(DiagnosticSpan {
                crate_root: crate_root.clone(),
                package_id: message.package_id.to_string(),
                file,
                start: usize::try_from(span.byte_start)
                    .expect("rustc source offsets fit into usize"),
                end: usize::try_from(span.byte_end).expect("rustc source offsets fit into usize"),
            });
        }
    }
    Ok(spans)
}
