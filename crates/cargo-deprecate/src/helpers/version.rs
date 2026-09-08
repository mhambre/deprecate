use semver::Version;

/// Parses abbreviated deprecation versions through the standard version parser.
pub(crate) fn parse(value: &str) -> Option<Version> {
    let suffix_at = value.find(['-', '+']).unwrap_or(value.len());
    let (core, suffix) = value.split_at(suffix_at);
    let normalized = match core.split('.').count() {
        1 => format!("{core}.0.0{suffix}"),
        2 => format!("{core}.0{suffix}"),
        _ => value.to_owned(),
    };
    Version::parse(&normalized).ok()
}

/// Validates both endpoints and their lifecycle order.
pub(crate) fn validate(since: &str, remove: Option<&str>) -> Result<(), String> {
    let since_version = parse(since).ok_or_else(|| format!("invalid `since` version `{since}`"))?;
    if let Some(remove) = remove {
        let remove_version =
            parse(remove).ok_or_else(|| format!("invalid `remove` version `{remove}`"))?;
        if remove_version <= since_version {
            return Err(format!(
                "`remove` ({remove}) must be later than `since` ({since})"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse, validate};

    #[test]
    fn normalizes_lifecycle_shorthand() {
        assert_eq!(parse("2").expect("version").to_string(), "2.0.0");
        assert_eq!(parse("2.1").expect("version").to_string(), "2.1.0");
    }

    #[test]
    fn rejects_reversed_lifecycle() {
        assert!(validate("2", Some("1.9")).is_err());
        assert!(validate("2", Some("2.0.0")).is_err());
    }
}
