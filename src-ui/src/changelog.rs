/// The bundled changelog, compiled in so the update history needs no network
/// and shows real content offline. Updating it means editing `CHANGELOG.md` at
/// the repo root and rebuilding.
const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

/// One released version and what changed in it.
#[derive(Clone, PartialEq)]
pub struct Release {
    pub version: String,
    /// Whatever followed the version on the heading line; empty when absent.
    pub date: String,
    pub changes: Vec<String>,
}

/// Parse `CHANGELOG.md`: a `## <version> — <date>` heading opens a release and
/// the `-` bullets under it are its changes. Anything else — the title, the
/// explanatory prose — is skipped, so the file stays readable as markdown.
///
/// Continuation lines of a wrapped bullet are folded back into it, since the
/// file is wrapped for reading rather than written one line per change.
pub fn releases() -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    for line in CHANGELOG.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            let (version, date) = match heading.split_once('—') {
                Some((version, date)) => (version.trim(), date.trim()),
                None => (heading.trim(), ""),
            };
            releases.push(Release {
                version: version.to_string(),
                date: date.to_string(),
                changes: Vec::new(),
            });
            continue;
        }
        let Some(current) = releases.last_mut() else { continue };
        if let Some(bullet) = trimmed.strip_prefix("- ") {
            current.changes.push(bullet.trim().to_string());
        } else if !trimmed.is_empty() && line.starts_with(' ') {
            if let Some(last) = current.changes.last_mut() {
                last.push(' ');
                last.push_str(trimmed);
            }
        }
    }
    releases
}

#[cfg(test)]
mod tests {
    use super::releases;

    #[test]
    fn parses_the_bundled_changelog() {
        let releases = releases();

        assert!(!releases.is_empty(), "changelog has no releases");
        let first = &releases[0];
        assert!(!first.version.is_empty());
        assert!(!first.changes.is_empty());
        // The prose above the first heading must not be read as a change.
        assert!(first.changes.iter().all(|c| !c.starts_with('#')));
    }

    #[test]
    fn folds_a_wrapped_bullet_into_one_change() {
        let parsed = releases();
        let wrapped = parsed
            .iter()
            .flat_map(|r| &r.changes)
            .any(|c| c.contains("per-instance mod list"));

        assert!(wrapped, "a wrapped bullet was not folded back together");
    }
}
