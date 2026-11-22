//! Links to file a bug report or feature request on GitHub, using the repository's issue
//! templates (`.github/ISSUE_TEMPLATE`). A bug report comes with the OS and Vikui version
//! filled in, since the web form can't tell them.

use url::Url;

/// The text the bug report's one box starts with: its template's, with this build's
/// details filled in. Keep it in step with `bug_report.yml`.
fn bug_report_text(os: &str, version: &str) -> String {
    format!(
        "**What happened**\n\n\n\
         **Steps to reproduce**\n1.\n\n\
         **What I expected**\n\n\n\
         ---\n\
         OS: {os}\n\
         Vikui version: {version}\n"
    )
}

/// The OS, its version and the CPU, e.g. "macOS 15.2 Sequoia (aarch64)".
fn os() -> String {
    let name =
        sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_owned());
    format!("{name} ({})", std::env::consts::ARCH)
}

/// A new issue in `repository` from `template`, with its `description` box prefilled.
fn new_issue(repository: &str, template: &str, description: Option<&str>) -> String {
    let fallback = format!("{}/issues/new", repository.trim_end_matches('/'));
    let Ok(mut url) = Url::parse(&fallback) else {
        return fallback;
    };
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("template", template);
        if let Some(description) = description {
            query.append_pair("description", description);
        }
    }
    url.into()
}

/// A new bug report, with this build's OS and version filled in.
pub fn bug_report() -> String {
    let text = bug_report_text(&os(), env!("CARGO_PKG_VERSION"));
    new_issue(env!("CARGO_PKG_REPOSITORY"), "bug_report.yml", Some(&text))
}

/// A new feature request.
pub fn feature_request() -> String {
    new_issue(env!("CARGO_PKG_REPOSITORY"), "feature_request.yml", None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefills_the_bug_report_with_the_os_and_version() {
        let url = new_issue(
            "https://github.com/someone/vikui",
            "bug_report.yml",
            Some(&bug_report_text("macOS 15.2 (aarch64)", "1.2.3")),
        );
        let url = Url::parse(&url).unwrap();
        assert_eq!(url.path(), "/someone/vikui/issues/new");
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        assert_eq!(query[0], ("template".into(), "bug_report.yml".into()));
        assert_eq!(query[1].0, "description");
        assert!(query[1].1.contains("OS: macOS 15.2 (aarch64)\n"));
        assert!(query[1].1.contains("Vikui version: 1.2.3\n"));
    }

    #[test]
    fn links_the_feature_request_template() {
        assert_eq!(
            new_issue(
                "https://github.com/someone/vikui/",
                "feature_request.yml",
                None
            ),
            "https://github.com/someone/vikui/issues/new?template=feature_request.yml"
        );
    }

    #[test]
    fn names_the_os() {
        assert!(os().ends_with(&format!("({})", std::env::consts::ARCH)));
    }

    /// The prefilled text must start like the template's, so a report from the app looks
    /// like one filed on GitHub.
    #[test]
    fn matches_the_bug_report_template() {
        let template = include_str!("../../../.github/ISSUE_TEMPLATE/bug_report.yml");
        let text = bug_report_text("", "");
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let line = line.trim_end();
            assert!(
                template.lines().any(|t| t.trim() == line.trim()),
                "{line:?} is not in bug_report.yml"
            );
        }
    }
}
