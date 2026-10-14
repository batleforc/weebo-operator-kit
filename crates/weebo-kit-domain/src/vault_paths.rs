//! Which KV paths an emitting object may write to: its namespace's subtree
//! `<pathPrefix>/<namespace>/`, or a path the instance's `pathGrants`
//! open to it. Shared by the secret stores and the admission webhooks, so
//! both refuse the same paths with the same message.
//!
//! Paths and grant patterns are compared segment by segment, never as
//! string prefixes. A pattern's segment is a literal, `+` (exactly one
//! segment) or, last only, `*` (one or more segments), and every pattern
//! holds at least one literal.

use std::fmt;

use crate::allow_list::namespace_matches;

/// One `pathGrants` entry, borrowed from the instance's spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathGrant<'a> {
    /// Namespace patterns (`name`, `*`, `team-*`) the grant opens `paths`
    /// to. Empty on a cluster-scoped grant.
    pub namespaces: &'a [String],
    /// The grant opens `paths` to cluster-scoped objects, and only to them.
    pub cluster_scoped: bool,
    pub paths: &'a [String],
}

/// Who writes: the emitting object, by scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Writer<'a> {
    Namespaced(&'a str),
    Cluster,
}

/// Why a path is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathDenied {
    Malformed {
        path: String,
        why: &'static str,
    },
    /// Out of the writer's subtree (`None`: a cluster-scoped writer naming
    /// no namespace has none), and no grant covers it.
    NotGranted {
        path: String,
        subtree: Option<String>,
        namespace: Option<String>,
    },
}

impl fmt::Display for PathDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathDenied::Malformed { path, why } => write!(f, "Vault path {path:?} {why}"),
            PathDenied::NotGranted {
                path,
                subtree,
                namespace,
            } => {
                match subtree {
                    Some(subtree) => write!(f, "Vault path {path:?} is outside {subtree:?}")?,
                    None => write!(f, "Vault path {path:?} names no namespace")?,
                }
                match namespace {
                    Some(ns) => write!(
                        f,
                        " and no pathGrants entry of the instance allows it for namespace {ns:?}"
                    ),
                    None => write!(
                        f,
                        " and no pathGrants entry of the instance allows it for cluster-scoped objects"
                    ),
                }
            }
        }
    }
}

impl std::error::Error for PathDenied {}

/// Why a grant is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantError(pub String);

impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GrantError {}

/// `path` without its leading `/`s. Refused when empty, or holding an
/// empty, `.` or `..` segment.
pub fn normalize(path: &str) -> Result<&str, PathDenied> {
    let normalized = path.trim_start_matches('/');
    let why = if normalized.is_empty() {
        Some("is empty")
    } else if normalized.split('/').any(str::is_empty) {
        Some("has an empty segment")
    } else if normalized.split('/').any(|s| s == "." || s == "..") {
        Some("has a \".\" or \"..\" segment")
    } else {
        None
    };
    match why {
        Some(why) => Err(PathDenied::Malformed {
            path: path.to_string(),
            why,
        }),
        None => Ok(normalized),
    }
}

/// `<prefix>/<namespace>/`, or `<namespace>/` with an empty prefix: the
/// subtree of a namespace's objects.
pub fn subtree(prefix: &str, namespace: &str) -> String {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        format!("{namespace}/")
    } else {
        format!("{prefix}/{namespace}/")
    }
}

/// `<prefix>/<namespace>/<name>`, the path of a target without one.
pub fn default_path(prefix: &str, namespace: &str, name: &str) -> String {
    format!("{}{name}", subtree(prefix, namespace))
}

/// Whether a grant covers `writer` writing the normalized `path`. An
/// invalid pattern covers nothing.
pub fn granted(grants: &[PathGrant<'_>], writer: Writer<'_>, path: &str) -> bool {
    grants.iter().any(|grant| {
        let applies = match writer {
            Writer::Namespaced(ns) => {
                !grant.cluster_scoped && grant.namespaces.iter().any(|p| namespace_matches(p, ns))
            }
            Writer::Cluster => grant.cluster_scoped,
        };
        applies
            && grant
                .paths
                .iter()
                .any(|pattern| check_pattern(pattern).is_ok() && pattern_matches(pattern, path))
    })
}

fn pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    let (fixed, rest) = match pattern.split_last() {
        Some((&"*", fixed)) => (fixed, true),
        _ => (pattern.as_slice(), false),
    };
    let lengths = if rest {
        path.len() > fixed.len()
    } else {
        path.len() == fixed.len()
    };
    lengths && fixed.iter().zip(&path).all(|(p, s)| *p == "+" || p == s)
}

fn check_pattern(pattern: &str) -> Result<(), String> {
    let segments: Vec<&str> = pattern.split('/').collect();
    let last = segments.len() - 1;
    for (i, segment) in segments.iter().enumerate() {
        let why = match *segment {
            "" => Some("has an empty segment (or a leading \"/\")"),
            "." | ".." => Some("has a \".\" or \"..\" segment"),
            "*" if i != last => Some("has \"*\" before its last segment"),
            "*" | "+" => None,
            s if s.contains(['*', '+']) => Some("mixes \"*\" or \"+\" with other characters"),
            _ => None,
        };
        if let Some(why) = why {
            return Err(format!("path pattern {pattern:?} {why}"));
        }
    }
    if segments.iter().all(|s| *s == "*" || *s == "+") {
        return Err(format!(
            "path pattern {pattern:?} has no literal segment: it would cover the whole mount"
        ));
    }
    Ok(())
}

/// Every grant names namespaces or is cluster-scoped (not both), and
/// lists at least one valid path pattern.
pub fn validate(grants: &[PathGrant<'_>]) -> Result<(), GrantError> {
    for (i, grant) in grants.iter().enumerate() {
        let error = |why: String| GrantError(format!("pathGrants[{i}]: {why}"));
        match (grant.namespaces.is_empty(), grant.cluster_scoped) {
            (true, false) => {
                return Err(error("names no namespace and is not clusterScoped".into()));
            }
            (false, true) => return Err(error("names namespaces and is clusterScoped".into())),
            _ => {}
        }
        if grant.namespaces.iter().any(String::is_empty) {
            return Err(error("has an empty namespace pattern".into()));
        }
        if grant.paths.is_empty() {
            return Err(error("lists no path".into()));
        }
        for pattern in grant.paths {
            check_pattern(pattern).map_err(error)?;
        }
    }
    Ok(())
}

/// The normalized `path` when `writer` may write it: under
/// `<prefix>/<namespace>/` (`namespace`: the writer's own, or the one a
/// cluster-scoped writer's target names), or covered by a grant.
pub fn check<'p>(
    prefix: &str,
    grants: &[PathGrant<'_>],
    writer: Writer<'_>,
    namespace: Option<&str>,
    path: &'p str,
) -> Result<&'p str, PathDenied> {
    let normalized = normalize(path)?;
    let subtree = namespace.map(|ns| subtree(prefix, ns));
    if subtree
        .as_deref()
        .is_some_and(|s| normalized.starts_with(s))
    {
        return Ok(normalized);
    }
    if granted(grants, writer, normalized) {
        return Ok(normalized);
    }
    Err(PathDenied::NotGranted {
        path: path.to_string(),
        subtree,
        namespace: match writer {
            Writer::Namespaced(ns) => Some(ns.to_string()),
            Writer::Cluster => None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn normalize_strips_leading_slashes_and_refuses_odd_segments() {
        assert_eq!(normalize("/a/b"), Ok("a/b"));
        assert_eq!(normalize("a/b"), Ok("a/b"));
        for bad in ["", "/", "a//b", "a/b/", "a/../b", "..", "a/./b"] {
            assert!(normalize(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_empty_prefix_is_skipped() {
        assert_eq!(subtree("weebo", "team-a"), "weebo/team-a/");
        assert_eq!(subtree("", "team-a"), "team-a/");
        assert_eq!(subtree("/weebo/", "team-a"), "weebo/team-a/");
        assert_eq!(default_path("", "team-a", "ci"), "team-a/ci");
        assert_eq!(default_path("weebo", "team-a", "ci"), "weebo/team-a/ci");
    }

    #[test]
    fn patterns_compare_segment_by_segment() {
        let cases = [
            ("batlehub/auth", "batlehub/auth", true),
            ("batlehub/auth", "batlehub/auth/x", false),
            ("batlehub/auth", "batlehub/authx", false),
            ("+/auth", "batlehub/auth", true),
            ("+/auth", "auth", false),
            ("+/auth", "a/b/auth", false),
            ("+/auth", "batlehub/auth/x", false),
            ("a/+/c", "a/b/c", true),
            ("a/+/c", "a/b/d", false),
            ("batlehub/*", "batlehub/auth", true),
            ("batlehub/*", "batlehub/a/b", true),
            ("batlehub/*", "batlehub", false),
            ("batlehub/*", "batlehubx/auth", false),
            ("+/auth/*", "x/auth/y/z", true),
            ("+/auth/*", "x/auth", false),
        ];
        for (pattern, path, expected) in cases {
            assert_eq!(
                pattern_matches(pattern, path),
                expected,
                "{pattern} ~ {path}"
            );
        }
    }

    #[test]
    fn validate_refuses_whole_mount_and_malformed_patterns() {
        let ns = s(&["auth"]);
        for bad in [
            "*", "+", "+/*", "+/+", "a/*/b", "/a", "a//b", "a/../b", "a/", "a*", "b+/c",
        ] {
            let paths = s(&[bad]);
            let grant = PathGrant {
                namespaces: &ns,
                cluster_scoped: false,
                paths: &paths,
            };
            assert!(validate(&[grant]).is_err(), "{bad}");
        }
        let paths = s(&["+/auth", "a/+/c/*", "batlehub/auth"]);
        let good = PathGrant {
            namespaces: &ns,
            cluster_scoped: false,
            paths: &paths,
        };
        assert_eq!(validate(&[good]), Ok(()));
        let no_paths = PathGrant { paths: &[], ..good };
        assert!(validate(&[no_paths]).is_err());
        let both = PathGrant {
            cluster_scoped: true,
            ..good
        };
        assert!(validate(&[both]).is_err());
        let neither = PathGrant {
            namespaces: &[],
            ..good
        };
        assert!(validate(&[neither]).is_err());
    }

    #[test]
    fn a_namespaced_writer_needs_its_subtree_or_a_grant() {
        let ns = s(&["auth", "team-*"]);
        let paths = s(&["batlehub/auth", "+/creds"]);
        let grants = [PathGrant {
            namespaces: &ns,
            cluster_scoped: false,
            paths: &paths,
        }];
        let ok = |ns: &str, path: &str| {
            check("weebo", &grants, Writer::Namespaced(ns), Some(ns), path).is_ok()
        };
        assert!(ok("other", "weebo/other/ci"));
        assert!(ok("other", "/weebo/other/ci"));
        assert!(!ok("other", "weebo/other-evil/ci"));
        assert!(!ok("other", "weebo/other/../auth/ci"));
        assert!(!ok("other", "batlehub/auth"));
        assert!(ok("auth", "batlehub/auth"));
        assert!(ok("team-a", "batlehub/auth"));
        assert!(ok("auth", "x/creds"));
        assert!(!ok("auth", "batlehub/auth/x"));
        let err = check(
            "weebo",
            &grants,
            Writer::Namespaced("other"),
            Some("other"),
            "x/y",
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            err,
            "Vault path \"x/y\" is outside \"weebo/other/\" and no pathGrants entry of the \
             instance allows it for namespace \"other\""
        );
    }

    #[test]
    fn an_invalid_pattern_covers_nothing() {
        let ns = s(&["*"]);
        let paths = s(&["*", "+/+"]);
        let grants = [PathGrant {
            namespaces: &ns,
            cluster_scoped: false,
            paths: &paths,
        }];
        assert!(!granted(&grants, Writer::Namespaced("a"), "x/y"));
    }

    #[test]
    fn a_cluster_writer_needs_a_cluster_scoped_grant() {
        let all = s(&["*"]);
        let paths = s(&["platform/+/credentials"]);
        let namespaced = [PathGrant {
            namespaces: &all,
            cluster_scoped: false,
            paths: &paths,
        }];
        let cluster = [PathGrant {
            namespaces: &[],
            cluster_scoped: true,
            paths: &paths,
        }];
        let path = "platform/x/credentials";
        assert!(check("weebo", &[], Writer::Cluster, Some("ns"), "weebo/ns/ci").is_ok());
        assert!(check("weebo", &[], Writer::Cluster, Some("ns"), path).is_err());
        assert!(check("weebo", &namespaced, Writer::Cluster, None, path).is_err());
        assert!(check("weebo", &cluster, Writer::Cluster, None, path).is_ok());
        assert!(check("weebo", &cluster, Writer::Namespaced("a"), Some("a"), path).is_err());
        assert_eq!(
            check("weebo", &[], Writer::Cluster, None, "x/y")
                .unwrap_err()
                .to_string(),
            "Vault path \"x/y\" names no namespace and no pathGrants entry of the instance \
             allows it for cluster-scoped objects"
        );
    }
}
