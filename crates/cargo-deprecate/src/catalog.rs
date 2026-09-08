use crate::helpers::version;
use crate::scan::Found;
use deprecate::{Deprecation, CATALOG_VERSION};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Serialize)]
pub struct Catalog {
    pub schema: u32,
    #[serde(rename = "crate")]
    pub package: String,
    pub version: String,
    pub deprecations: Vec<Deprecation>,
}

/// Serializes one package's declarations into catalog schema v1.
pub fn render(package: &str, version: &str, entries: &[Found]) -> Result<String, String> {
    let catalog = Catalog {
        schema: CATALOG_VERSION,
        package: package.to_owned(),
        version: version.to_owned(),
        deprecations: entries
            .iter()
            .filter(|entry| entry.package == package)
            .map(|entry| entry.data.clone())
            .collect(),
    };
    let body = serde_json::to_string_pretty(&catalog)
        .map_err(|error| format!("failed to serialize deprecation catalog: {error}"))?;
    Ok(format!("{body}\n"))
}

/// Reads a supported catalog and rejects unknown schema versions.
pub fn read(path: &Path) -> Result<Catalog, String> {
    let input = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let catalog: Catalog = serde_json::from_str(&input)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if catalog.schema != CATALOG_VERSION {
        return Err(format!(
            "unsupported catalog schema {}; this version supports {CATALOG_VERSION}",
            catalog.schema
        ));
    }
    for entry in &catalog.deprecations {
        version::validate(&entry.since, entry.remove.as_deref())
            .map_err(|error| format!("invalid catalog entry `{}`: {error}", entry.name))?;
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::{read, render};
    use crate::scan::Found;
    use deprecate::{Deprecation, Kind};
    use std::path::PathBuf;

    #[test]
    fn renders_stable_catalog() {
        let entries = vec![Found {
            package_id: "demo 1.2.0".to_owned(),
            package: "demo".to_owned(),
            package_version: "1.2.0".to_owned(),
            data: Deprecation {
                name: "old".to_owned(),
                kind: Kind::Item,
                since: "1.1".to_owned(),
                remove: Some("2".to_owned()),
                replacement: Some("new".to_owned()),
                reason: None,
                migrate: None,
            },
            file: PathBuf::from("src/lib.rs"),
            line: 1,
            offset: 0,
        }];
        let output = render("demo", "1.2.0", &entries).expect("catalog serializes");
        assert!(output.contains("\"schema\": 1"));
        assert!(output.contains("\"name\": \"old\""));
    }

    #[test]
    fn rejects_unknown_schema() {
        // Reading a real file covers the same boundary used for dependency catalogs.
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("deprecations.json");
        std::fs::write(
            &path,
            r#"{"schema":999,"crate":"demo","version":"1.0.0","deprecations":[]}"#,
        )
        .expect("fixture is written");
        let error = read(&path).expect_err("schema should be rejected");
        assert!(error.contains("unsupported catalog schema 999"));
    }
}
