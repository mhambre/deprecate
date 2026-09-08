use std::path::Path;

/// Displays paths inside the workspace without the absolute prefix.
pub(crate) fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Collapses source snippets for one-line migration previews.
pub(crate) fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
