use crate::catalog;
use crate::helpers::output::{relative_path, single_line};
use crate::helpers::version::parse as parse_version;
use crate::migrate;
use crate::scan::{self, Found};
use crate::workspace;
use clap::error::ErrorKind;
use clap::{Parser, Subcommand, ValueEnum};
use deprecate::{Deprecation, Kind};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "Cargo Deprecate",
    version,
    about = "Structured Rust deprecation lifecycle tooling"
)]
struct Cli {
    /// Include every package in the resolved dependency graph.
    #[arg(long, global = true)]
    workspace: bool,

    /// Include every package in the resolved dependency graph.
    #[arg(long, global = true)]
    dependencies: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Inventory structured API and feature deprecations.
    List,
    /// Enforce lifecycle policy for CI.
    Check {
        /// Treat a lifecycle condition as an error.
        #[arg(long, value_enum)]
        deny: Vec<Policy>,
    },
    /// Apply supported downstream call migrations.
    Fix {
        /// Print the migration plan without writing files.
        #[arg(long)]
        dry_run: bool,
        /// Ignore deprecation declarations from dependencies.
        #[arg(long)]
        workspace_only: bool,
    },
    /// Write the stable deprecations.json exchange format.
    Catalog {
        /// Write a single-package catalog to this path.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Compare current declarations with an older catalog.
    Diff {
        /// Catalog containing the comparison baseline.
        #[arg(long)]
        from: PathBuf,
    },
}

#[derive(Clone, Copy, Eq, PartialEq, ValueEnum)]
enum Policy {
    Overdue,
    MissingReplacement,
    All,
}

/// Normalizes Cargo's argv shape and dispatches the selected subcommand.
pub fn run(mut arguments: Vec<String>) -> Result<(), String> {
    if arguments.first().is_some_and(|value| value == "deprecate") {
        arguments.remove(0);
    }
    arguments.insert(0, "cargo deprecate".to_owned());
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            print!("{error}");
            return Ok(());
        }
        Err(error) => {
            let message = error.to_string();
            return Err(message
                .strip_prefix("error: ")
                .unwrap_or(&message)
                .trim_end()
                .to_owned());
        }
    };
    let include_dependencies = cli.workspace || cli.dependencies;
    match cli.command.unwrap_or(Command::List) {
        Command::List => list(include_dependencies),
        Command::Check { deny } => check(include_dependencies, &deny),
        Command::Fix {
            dry_run,
            workspace_only,
        } => fix(dry_run, workspace_only),
        Command::Catalog { output } => write_catalog(output.as_deref()),
        Command::Diff { from } => diff(&from),
    }
}

/// Prints the deprecation inventory and migratable usage counts.
fn list(include_dependencies: bool) -> Result<(), String> {
    let workspace = workspace::load()?;
    let entries = scan::workspace(&workspace, include_dependencies)?;
    if entries.is_empty() {
        println!("No structured deprecations found.");
        return Ok(());
    }
    let resolver = crate::resolve::Resolver::load(&workspace)?;
    println!("Deprecated APIs and features:\n");
    let mut usage_total = 0;
    for entry in &entries {
        let feature = if entry.data.kind == Kind::Feature {
            "feature "
        } else {
            ""
        };
        println!("  {}: {feature}{}", entry.package, entry.data.name);
        println!("    since       {}", entry.data.since);
        if let Some(remove) = &entry.data.remove {
            println!("    remove      {remove}");
        }
        if let Some(replacement) = &entry.data.replacement {
            println!("    replacement {replacement}");
        }
        if entry.data.kind == Kind::Item {
            let usages = migrate::count_usages(&resolver, entry)?;
            usage_total += usages;
            println!("    usages      {usages}");
        }
        println!(
            "    location    {}:{}",
            relative_path(&workspace.root, &entry.file),
            entry.line
        );
        println!();
    }
    println!("{} deprecations", entries.len());
    println!("{usage_total} candidate usages (fix confirms compiler diagnostics)");
    Ok(())
}

/// Evaluates lifecycle and feature declarations against CI policy.
fn check(include_dependencies: bool, denied: &[Policy]) -> Result<(), String> {
    let deny_overdue = denied.contains(&Policy::Overdue) || denied.contains(&Policy::All);
    let deny_missing =
        denied.contains(&Policy::MissingReplacement) || denied.contains(&Policy::All);
    let workspace = workspace::load()?;
    let entries = scan::workspace(&workspace, include_dependencies)?;
    let mut violations = 0;
    for entry in &entries {
        if entry.data.kind == Kind::Feature {
            let package = workspace
                .packages
                .iter()
                .find(|package| package.id == entry.package_id);
            if package.is_some_and(|package| !package.declared_features.contains(&entry.data.name))
            {
                eprintln!(
                    "error[deprecate::unknown_feature]: `{}` is not declared by package `{}`",
                    entry.data.name, entry.package
                );
                details(entry, &workspace.root);
                violations += 1;
            }
            if let Some(replacement) = &entry.data.replacement {
                if package.is_some_and(|package| !package.declared_features.contains(replacement)) {
                    eprintln!(
                        "error[deprecate::unknown_replacement]: replacement feature `{replacement}` is not declared by package `{}`",
                        entry.package
                    );
                    details(entry, &workspace.root);
                    violations += 1;
                }
            }
        }
        let current = parse_version(&entry.package_version);
        let due = entry
            .data
            .remove
            .as_deref()
            .and_then(parse_version)
            .zip(current)
            .is_some_and(|(remove, current)| current >= remove);
        if due {
            let level = if deny_overdue { "error" } else { "warning" };
            eprintln!(
                "{level}[deprecate::removal_due]: `{}` was scheduled for removal in {}",
                entry.data.name,
                entry.data.remove.as_deref().unwrap_or_default()
            );
            details(entry, &workspace.root);
            violations += usize::from(deny_overdue);
        }
        if entry.data.replacement.is_none() && entry.data.migrate.is_none() {
            let level = if deny_missing { "error" } else { "warning" };
            eprintln!(
                "{level}[deprecate::missing_replacement]: `{}` has no replacement or migration recipe",
                entry.data.name
            );
            details(entry, &workspace.root);
            violations += usize::from(deny_missing);
        }
        if entry.data.kind == Kind::Feature
            && workspace
                .enabled_features
                .get(&entry.package_id)
                .is_some_and(|features| features.contains(&entry.data.name))
        {
            eprintln!(
                "warning[deprecate::feature_enabled]: dependency `{}` enables deprecated feature `{}`",
                entry.package, entry.data.name
            );
            details(entry, &workspace.root);
        }
    }
    if violations > 0 {
        Err(format!("{violations} deprecation policy violation(s)"))
    } else {
        println!(
            "deprecation policy check passed ({} entries)",
            entries.len()
        );
        Ok(())
    }
}

/// Builds and optionally applies a compiler-confirmed migration plan.
fn fix(dry_run: bool, workspace_only: bool) -> Result<(), String> {
    let workspace = workspace::load()?;
    let entries = scan::workspace(&workspace, !workspace_only)?;
    let spans = workspace::deprecated_spans(&workspace)?;
    let resolver = crate::resolve::Resolver::load(&workspace)?;
    let (changes, manual) = migrate::plan(&resolver, &entries, &spans)?;
    let edit_count: usize = changes.iter().map(|change| change.edits.len()).sum();
    if edit_count == 0 {
        println!("No automatically migratable usages found.");
        if manual > 0 {
            println!("{manual} usages require manual migration");
        }
        return Ok(());
    }
    println!("Migrating {edit_count} usages...\n");
    for change in &changes {
        println!(" {}", relative_path(&workspace.root, &change.path));
        for edit in &change.edits {
            println!(
                "   {}:{} -> {}",
                edit.line,
                single_line(&edit.before),
                single_line(&edit.after)
            );
        }
        println!();
    }
    if dry_run {
        println!("{edit_count} migrations planned; no files changed");
    } else {
        migrate::apply(&changes, &workspace)?;
        println!("{edit_count} usages migrated automatically");
    }
    if manual > 0 {
        println!("{manual} usages require manual migration");
    }
    Ok(())
}

/// Writes one catalog for each publishable workspace package.
fn write_catalog(output: Option<&Path>) -> Result<(), String> {
    let workspace = workspace::load()?;
    let entries = scan::workspace(&workspace, false)?;
    let packages: Vec<_> = workspace
        .packages
        .iter()
        .filter(|package| package.workspace && package.name != "deprecate-macros")
        .collect();
    if output.is_some() && packages.len() != 1 {
        return Err("--output requires a workspace with one publishable package".to_owned());
    }
    for package in packages {
        let path = output.map_or_else(|| package.root.join("deprecations.json"), Path::to_path_buf);
        let rendered = catalog::render(&package.name, &package.version, &entries)?;
        fs::write(&path, rendered)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        println!("wrote {}", relative_path(&workspace.root, &path));
    }
    Ok(())
}

/// Compares current source declarations with a previous catalog.
fn diff(from: &Path) -> Result<(), String> {
    let previous = catalog::read(from)?;
    let workspace = workspace::load()?;
    let current: Vec<_> = scan::workspace(&workspace, false)?
        .into_iter()
        .filter(|entry| entry.package == previous.package)
        .map(|entry| entry.data)
        .collect();
    if !workspace
        .packages
        .iter()
        .any(|package| package.workspace && package.name == previous.package)
    {
        return Err(format!(
            "catalog package `{}` is not in the current workspace",
            previous.package
        ));
    }
    let previous_by_key = keyed(&previous.deprecations);
    let current_by_key = keyed(&current);
    println!("Deprecation changes since {}\n", from.display());
    let mut changes = 0;
    for (key, entry) in &current_by_key {
        match previous_by_key.get(key) {
            None => {
                println!("+ {}", entry.name);
                changes += 1;
            }
            Some(old) if *old != *entry => {
                println!("~ {}", entry.name);
                if old.remove != entry.remove {
                    println!(
                        "    removal {} -> {}",
                        old.remove.as_deref().unwrap_or("none"),
                        entry.remove.as_deref().unwrap_or("none")
                    );
                }
                if old.replacement != entry.replacement {
                    println!(
                        "    replacement {} -> {}",
                        old.replacement.as_deref().unwrap_or("none"),
                        entry.replacement.as_deref().unwrap_or("none")
                    );
                }
                changes += 1;
            }
            Some(_) => {}
        }
    }
    for (key, entry) in &previous_by_key {
        if !current_by_key.contains_key(key) {
            println!("- {}", entry.name);
            changes += 1;
        }
    }
    println!("\n{changes} changes");
    Ok(())
}

/// Indexes entries by kind and name for order-independent catalog diffs.
fn keyed(entries: &[Deprecation]) -> BTreeMap<(u8, &str), &Deprecation> {
    entries
        .iter()
        .map(|entry| {
            let kind = u8::from(entry.kind != Kind::Item);
            ((kind, entry.name.as_str()), entry)
        })
        .collect()
}

/// Prints the common diagnostic details for a lifecycle finding.
fn details(entry: &Found, root: &Path) {
    eprintln!("  --> {}:{}", relative_path(root, &entry.file), entry.line);
    eprintln!("      deprecated: {}", entry.data.since);
    if let Some(remove) = &entry.data.remove {
        eprintln!("      removal:    {remove}");
    }
    if let Some(replacement) = &entry.data.replacement {
        eprintln!("      replacement: {replacement}");
    }
    eprintln!("      current:     {}\n", entry.package_version);
}
