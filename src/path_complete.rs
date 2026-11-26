//! Lexical path normalization, project folder naming, and directory Tab-completion.
//!
//! Pure filesystem helpers with no tmux dependencies so forms and the composer
//! can offer terminal-like path completion (`~`, `./`, `../`, common prefix,
//! hidden-dot filtering, and symlink-aware directory matching).

use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

/// A single directory candidate produced by [`complete_directory`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathCompletionEntry {
    /// Short display label relative to the searched parent (always ends with `/`).
    pub display: String,
    /// Full text to substitute into the directory input (preserves user prefix like `~/`).
    pub full_input: String,
    /// Whether the entry on disk is a symbolic link pointing to a directory.
    pub is_symlink: bool,
}

/// Directory completion candidates and shared longest-common-prefix expansion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathCompletionResult {
    pub entries: Vec<PathCompletionEntry>,
    pub common_prefix: String,
}

/// Normalize `.` and `..` components lexically without requiring `path` to exist.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut parts: Vec<Component<'_>> = Vec::new();
    let mut has_root = false;

    for comp in path.components() {
        match comp {
            Component::Prefix(p) => {
                parts.push(Component::Prefix(p));
            }
            Component::RootDir => {
                has_root = true;
                parts.push(Component::RootDir);
            }
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(Component::Normal(_)) => {
                    parts.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                Some(Component::ParentDir) | None => {
                    if !has_root {
                        parts.push(Component::ParentDir);
                    }
                }
                Some(Component::CurDir) => {}
            },
            Component::Normal(seg) => parts.push(Component::Normal(seg)),
        }
    }

    if parts.is_empty() {
        return PathBuf::from(".");
    }
    let mut out = PathBuf::new();
    for comp in parts {
        out.push(comp.as_os_str());
    }
    out
}

/// Resolve user input (`~`, `~/...`, relative, or absolute) against `base`.
pub fn resolve_dir_path(raw: &str, base: &Path) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return normalize_lexical(base);
    }
    if trimmed == "~" {
        return env::home_dir()
            .map(|h| normalize_lexical(&h))
            .unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = env::home_dir() {
            return if rest.is_empty() {
                normalize_lexical(&home)
            } else {
                normalize_lexical(&home.join(rest))
            };
        }
        return normalize_lexical(Path::new(trimmed));
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        normalize_lexical(candidate)
    } else {
        normalize_lexical(&base.join(candidate))
    }
}

/// Format `path` concisely for display/editing, collapsing `$HOME` to `~`.
pub fn display_short_path(path: &Path) -> String {
    let norm = normalize_lexical(path);
    if let Some(home) = env::home_dir() {
        let norm_home = normalize_lexical(&home);
        if let Ok(rel) = norm
            .strip_prefix(&norm_home)
            .or_else(|_| path.strip_prefix(&home))
        {
            return if rel.as_os_str().is_empty() {
                "~".to_owned()
            } else {
                format!("~/{}", rel.display())
            };
        }
    }
    norm.display().to_string()
}

/// Derive a default window or session name from the directory's folder name.
///
/// When `sanitize_session` is `true`, replaces `.` and `:` with `-` to satisfy
/// tmux session naming constraints. Falls back to `"workspace"` if empty.
pub fn folder_name_from_path(path: &Path, sanitize_session: bool) -> String {
    let norm = normalize_lexical(path);
    let base = norm
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cleaned: String = base
        .trim()
        .chars()
        .map(|c| match c {
            '.' | ':' if sanitize_session => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "workspace".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Complete subdirectories matching `raw` (resolved relative to `base`).
///
/// - Only directories (including symlinks to directories) are returned.
/// - Dot-directories are hidden unless the basename prefix begins with `.`.
/// - Matches are case-insensitive, prioritizing exact-case prefix matches.
/// - `common_prefix` equals the single match's `full_input` (with trailing `/`)
///   or the shared prefix across multiple matches.
pub fn complete_directory(raw: &str, base: &Path) -> PathCompletionResult {
    if raw == "~" {
        let mut res = complete_in_parts("~/", "", base);
        if res.common_prefix.is_empty() {
            res.common_prefix = "~/".to_owned();
        }
        return res;
    }

    if raw == ".." {
        let parent_check = complete_in_parts("", "..", base);
        if parent_check.entries.is_empty() {
            let mut res = complete_in_parts("../", "", base);
            if res.common_prefix.is_empty() {
                res.common_prefix = "../".to_owned();
            }
            if res.entries.is_empty() {
                res.entries.push(PathCompletionEntry {
                    display: "../".to_owned(),
                    full_input: "../".to_owned(),
                    is_symlink: false,
                });
            }
            return res;
        }
        return parent_check;
    }

    if raw == "." {
        let dot_check = complete_in_parts("", ".", base);
        if dot_check.entries.is_empty() {
            let mut res = complete_in_parts("./", "", base);
            if res.common_prefix.is_empty() {
                res.common_prefix = "./".to_owned();
            }
            return res;
        }
        return dot_check;
    }

    let (dir_part, prefix) = match raw.rfind('/') {
        Some(idx) => (&raw[..=idx], &raw[idx + 1..]),
        None => ("", raw),
    };
    complete_in_parts(dir_part, prefix, base)
}

fn complete_in_parts(dir_part: &str, prefix: &str, base: &Path) -> PathCompletionResult {
    let search_dir = if dir_part.is_empty() {
        resolve_dir_path(".", base)
    } else {
        resolve_dir_path(dir_part, base)
    };
    let Ok(read_dir) = fs::read_dir(&search_dir) else {
        return PathCompletionResult::default();
    };

    let show_hidden = prefix.starts_with('.');
    let prefix_lower = prefix.to_lowercase();
    let mut matched: Vec<(bool, String, String, bool)> = Vec::new();

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name_os = entry.file_name();
        let name = name_os.to_string_lossy();
        if name == "." || name == ".." {
            continue;
        }
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let name_lower = name.to_lowercase();
        if !name_lower.starts_with(&prefix_lower) {
            continue;
        }
        let exact_case = name.starts_with(prefix);
        let is_symlink = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false);
        matched.push((exact_case, name_lower, name.into_owned(), is_symlink));
    }

    matched.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });

    if matched.is_empty() {
        return PathCompletionResult::default();
    }

    let entries: Vec<PathCompletionEntry> = matched
        .iter()
        .map(|(_, _, name, is_symlink)| PathCompletionEntry {
            display: format!("{name}/"),
            full_input: format!("{dir_part}{name}/"),
            is_symlink: *is_symlink,
        })
        .collect();

    let common_prefix = if entries.len() == 1 {
        entries[0].full_input.clone()
    } else {
        let first_chars: Vec<char> = matched[0].2.chars().collect();
        let mut lcp_len = first_chars.len();
        for (_, _, name, _) in matched.iter().skip(1) {
            let other_chars: Vec<char> = name.chars().collect();
            let shared = first_chars
                .iter()
                .zip(other_chars.iter())
                .take(lcp_len)
                .take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                .count();
            lcp_len = shared;
        }
        let prefix_chars = prefix.chars().count();
        if lcp_len <= prefix_chars {
            format!("{dir_part}{prefix}")
        } else {
            let shared_name: String = first_chars.into_iter().take(lcp_len).collect();
            format!("{dir_part}{shared_name}")
        }
    };

    PathCompletionResult {
        entries,
        common_prefix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lexical_normalization_and_folder_names_handle_dot_colon_and_root() {
        assert_eq!(
            normalize_lexical(Path::new("/a/b/../c/./d/")),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(
            normalize_lexical(Path::new("foo/bar/../baz")),
            PathBuf::from("foo/baz")
        );
        assert_eq!(
            normalize_lexical(Path::new("../foo")),
            PathBuf::from("../foo")
        );
        assert_eq!(normalize_lexical(Path::new("/..")), PathBuf::from("/"));

        assert_eq!(
            folder_name_from_path(Path::new("/Users/dev/my.app:v2/"), true),
            "my-app-v2"
        );
        assert_eq!(
            folder_name_from_path(Path::new("/Users/dev/my.app:v2/"), false),
            "my.app:v2"
        );
        assert_eq!(folder_name_from_path(Path::new("/"), true), "workspace");
    }

    #[test]
    fn resolve_dir_path_expands_tilde_and_relative_segments() {
        let base = Path::new("/tmp/root/proj");
        assert_eq!(resolve_dir_path("", base), PathBuf::from("/tmp/root/proj"));
        assert_eq!(
            resolve_dir_path("../other", base),
            PathBuf::from("/tmp/root/other")
        );
        assert_eq!(
            resolve_dir_path("./sub/", base),
            PathBuf::from("/tmp/root/proj/sub")
        );
        assert_eq!(
            resolve_dir_path("/var/log", base),
            PathBuf::from("/var/log")
        );

        if let Some(home) = env::home_dir() {
            let norm_home = normalize_lexical(&home);
            assert_eq!(resolve_dir_path("~", base), norm_home);
            assert_eq!(resolve_dir_path("~/", base), norm_home);
            assert_eq!(
                resolve_dir_path("~/projects/justmux", base),
                normalize_lexical(&home.join("projects/justmux"))
            );
            assert_eq!(display_short_path(&home), "~");
            assert_eq!(
                display_short_path(&home.join("projects/justmux")),
                "~/projects/justmux"
            );
        }
    }

    #[test]
    fn complete_directory_filters_dirs_hidden_symlinks_and_computes_lcp() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir(root.join("alpha")).expect("alpha");
        fs::create_dir(root.join("project-api")).expect("project-api");
        fs::create_dir(root.join("project-web")).expect("project-web");
        fs::create_dir(root.join("ProjectDocs")).expect("ProjectDocs");
        fs::create_dir(root.join(".git")).expect("hidden");
        fs::write(root.join("project-notes.txt"), "hi").expect("file");

        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("alpha"), root.join("project-link"))
            .expect("symlink dir");

        // Empty prefix omits files & hidden dirs.
        let all = complete_directory("", root);
        let names: Vec<&str> = all.entries.iter().map(|e| e.display.as_str()).collect();
        assert!(!names.contains(&"project-notes.txt"));
        assert!(!names.contains(&".git/"));
        assert!(names.contains(&"alpha/"));
        assert!(names.contains(&"project-api/"));

        // Hidden prefix reveals dot-dirs.
        let hidden = complete_directory(".", root);
        assert_eq!(hidden.entries.len(), 1);
        assert_eq!(hidden.entries[0].display, ".git/");
        assert_eq!(hidden.common_prefix, ".git/");

        // Single match gets trailing `/`.
        let single = complete_directory("al", root);
        assert_eq!(single.entries.len(), 1);
        assert_eq!(single.entries[0].full_input, "alpha/");
        assert_eq!(single.common_prefix, "alpha/");

        // Multi-match case-insensitive sorts exact-case prefix first & shares LCP `project-`.
        let multi = complete_directory("project-", root);
        assert!(multi.entries.iter().any(|e| e.display == "project-api/"));
        assert!(multi.entries.iter().any(|e| e.display == "project-web/"));
        #[cfg(unix)]
        {
            let sym = multi
                .entries
                .iter()
                .find(|e| e.display == "project-link/")
                .expect("symlink dir candidate");
            assert!(sym.is_symlink);
        }

        let lcp_test = complete_directory("pro", root);
        assert_eq!(lcp_test.entries.len(), 4);
        assert_eq!(lcp_test.common_prefix, "project");

        // Relative `./` preserves prefix.
        let rel = complete_directory("./al", root);
        assert_eq!(rel.common_prefix, "./alpha/");

        // Non-existent dir yields empty.
        let missing = complete_directory("nope/", root);
        assert!(missing.entries.is_empty());
        assert!(missing.common_prefix.is_empty());
    }
}
