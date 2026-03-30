use crate::core::workspace::PackageEntry;

#[derive(Debug, Default, Clone)]
pub struct PackageFilter {
    pub only: Vec<String>,
    pub exclude: Vec<String>,
    pub groups: Vec<String>,
}

impl PackageFilter {
    pub fn is_empty(&self) -> bool {
        self.only.is_empty() && self.exclude.is_empty() && self.groups.is_empty()
    }

    pub fn matches(&self, pkg: &PackageEntry) -> bool {
        // --only takes highest precedence
        if !self.only.is_empty() {
            return self.only.iter().any(|o| o == &pkg.name);
        }
        // --group filters by tag
        if !self.groups.is_empty() && !self.groups.iter().any(|g| pkg.groups.contains(g)) {
            return false;
        }
        // --exclude removes packages
        if !self.exclude.is_empty() {
            return !self.exclude.iter().any(|e| e == &pkg.name);
        }
        true
    }

    pub fn apply<'a>(&self, packages: &'a [PackageEntry]) -> Vec<&'a PackageEntry> {
        packages.iter().filter(|pkg| self.matches(pkg)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pkg(name: &str, groups: Vec<&str>) -> PackageEntry {
        PackageEntry {
            name: name.to_string(),
            url: format!("https://example.com/{}.git", name),
            branch: None,
            remote: None,
            sync_strategy: None,
            groups: groups.into_iter().map(String::from).collect(),
            hooks: crate::core::workspace::WorkspaceHooks::default(),
        }
    }

    #[test]
    fn test_empty_filter_matches_all() {
        let filter = PackageFilter::default();
        let pkg = make_pkg("api", vec!["backend"]);
        assert!(filter.matches(&pkg));
    }

    #[test]
    fn test_only_filter() {
        let filter = PackageFilter {
            only: vec!["api".into()],
            ..Default::default()
        };
        assert!(filter.matches(&make_pkg("api", vec![])));
        assert!(!filter.matches(&make_pkg("web", vec![])));
    }

    #[test]
    fn test_exclude_filter() {
        let filter = PackageFilter {
            exclude: vec!["web".into()],
            ..Default::default()
        };
        assert!(filter.matches(&make_pkg("api", vec![])));
        assert!(!filter.matches(&make_pkg("web", vec![])));
    }

    #[test]
    fn test_group_filter() {
        let filter = PackageFilter {
            groups: vec!["backend".into()],
            ..Default::default()
        };
        assert!(filter.matches(&make_pkg("api", vec!["backend", "rust"])));
        assert!(!filter.matches(&make_pkg("web", vec!["frontend"])));
    }

    #[test]
    fn test_group_filter_multiple_groups() {
        let filter = PackageFilter {
            groups: vec!["backend".into(), "frontend".into()],
            ..Default::default()
        };
        assert!(filter.matches(&make_pkg("api", vec!["backend"])));
        assert!(filter.matches(&make_pkg("web", vec!["frontend"])));
        assert!(!filter.matches(&make_pkg("infra", vec!["ops"])));
    }

    #[test]
    fn test_only_overrides_group_and_exclude() {
        let filter = PackageFilter {
            only: vec!["api".into()],
            groups: vec!["frontend".into()],
            exclude: vec!["api".into()],
        };
        assert!(filter.matches(&make_pkg("api", vec!["backend"])));
        assert!(!filter.matches(&make_pkg("web", vec!["frontend"])));
    }

    #[test]
    fn test_group_and_exclude_combined() {
        let filter = PackageFilter {
            groups: vec!["backend".into()],
            exclude: vec!["legacy".into()],
            ..Default::default()
        };
        assert!(filter.matches(&make_pkg("api", vec!["backend"])));
        assert!(!filter.matches(&make_pkg("legacy", vec!["backend"])));
        assert!(!filter.matches(&make_pkg("web", vec!["frontend"])));
    }

    #[test]
    fn test_apply_filters_list() {
        let pkgs = vec![
            make_pkg("api", vec!["backend"]),
            make_pkg("web", vec!["frontend"]),
            make_pkg("lib", vec!["backend", "shared"]),
        ];
        let filter = PackageFilter {
            groups: vec!["backend".into()],
            ..Default::default()
        };
        let result = filter.apply(&pkgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "api");
        assert_eq!(result[1].name, "lib");
    }
}
