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

/// The releases of the bundled changelog, newest first as the file lists them.
pub fn releases() -> Vec<Release> {
    parse(CHANGELOG)
}

/// Split a `## <version> <sep> <date>` heading. An em dash is what the file
/// uses, but an en dash or a spaced hyphen is what a future editor is likely to
/// type, and a bare hyphen can't be the separator because dates contain them.
fn split_heading(heading: &str) -> (&str, &str) {
    for separator in ["—", "–", " - "] {
        if let Some((version, date)) = heading.split_once(separator) {
            return (version.trim(), date.trim());
        }
    }
    (heading.trim(), "")
}

/// Parse a changelog: a `## <version> — <date>` heading opens a release and the
/// `-` bullets under it are its changes. Anything else — the title, the
/// explanatory prose — is skipped, so the file stays readable as markdown.
///
/// A line that continues the previous bullet is folded back into it, whether or
/// not it is indented: the file is wrapped for reading rather than written one
/// line per change, and an editor reflowing it will not indent by hand.
fn parse(text: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    // Whether the previous line left a bullet open to continue. A blank line or
    // a heading closes it, so loose prose is never folded into a change.
    let mut open_bullet = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            open_bullet = false;
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            let (version, date) = split_heading(heading);
            releases.push(Release {
                version: version.to_string(),
                date: date.to_string(),
                changes: Vec::new(),
            });
            open_bullet = false;
            continue;
        }
        if trimmed.starts_with('#') {
            open_bullet = false;
            continue;
        }
        let Some(current) = releases.last_mut() else { continue };
        if let Some(bullet) = trimmed.strip_prefix("- ") {
            current.changes.push(bullet.trim().to_string());
            open_bullet = true;
        } else if open_bullet {
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
    use super::{parse, releases};

    #[test]
    fn parses_the_bundled_changelog() {
        let releases = releases();

        assert!(!releases.is_empty(), "changelog has no releases");
        let first = &releases[0];
        assert!(!first.version.is_empty());
        assert!(!first.date.is_empty(), "heading separator was not recognised");
        assert!(!first.changes.is_empty());
        // The prose above the first heading must not be read as a change.
        assert!(first.changes.iter().all(|c| !c.starts_with('#')));
    }

    #[test]
    fn folds_a_wrapped_bullet_whether_or_not_it_is_indented() {
        let parsed = parse("## 1.0.0 — 2026-01-01\n- indented\n  continuation\n- lazy\ncontinuation\n");

        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].changes,
            vec!["indented continuation", "lazy continuation"]
        );
    }

    #[test]
    fn a_blank_line_closes_a_bullet() {
        let parsed = parse("## 1.0.0 — 2026-01-01\n- one\n\nloose prose\n");

        assert_eq!(parsed[0].changes, vec!["one"]);
    }

    #[test]
    fn accepts_the_separators_an_editor_is_likely_to_type() {
        for heading in ["## 1.0.0 — 2026-01-01", "## 1.0.0 – 2026-01-01", "## 1.0.0 - 2026-01-01"] {
            let parsed = parse(&format!("{heading}\n- change\n"));

            assert_eq!(parsed[0].version, "1.0.0", "{heading}");
            assert_eq!(parsed[0].date, "2026-01-01", "{heading}");
        }
    }

    #[test]
    fn prose_before_the_first_heading_belongs_to_no_release() {
        let parsed = parse("# Changelog\n\nHow to read this file.\n\n## 1.0.0 — 2026-01-01\n- change\n");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].changes, vec!["change"]);
    }
}
