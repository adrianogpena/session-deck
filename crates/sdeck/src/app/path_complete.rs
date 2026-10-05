//! Path suggestions for the prompts that ask for a path: the entries of what has been typed so far.

use sdeck_core::discovery::path_utils::expand_input_path;

const MAX_SUGGESTIONS: usize = 50;

/// Where the typed value splits: the folder part (with its trailing separator) and the name prefix.
fn split(value: &str) -> (&str, &str) {
    let value = value.strip_prefix('"').unwrap_or(value);
    match value.rfind(['/', '\\']) {
        Some(i) => value.split_at(i + 1),
        None => ("", value),
    }
}

/// Entry names of the typed folder part that start with the typed prefix, case-insensitive. Folders
/// come first and end in `/`; files are listed only when `files` is set. Hidden entries only show
/// once the prefix starts with a dot.
pub(super) fn path_suggestions(value: &str, files: bool) -> Vec<String> {
    let (dir, prefix) = split(value);
    if dir.is_empty() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(expand_input_path(dir)) else {
        return Vec::new();
    };
    let prefix = prefix.to_lowercase();
    let mut found: Vec<(bool, String)> = entries
        .flatten()
        .filter_map(|e| {
            let is_dir = e.path().is_dir();
            (is_dir || files).then(|| (is_dir, e.file_name().into_string().ok()))
        })
        .filter_map(|(is_dir, name)| Some((is_dir, name?)))
        .filter(|(_, n)| {
            n.to_lowercase().starts_with(&prefix) && (prefix.starts_with('.') || !n.starts_with('.'))
        })
        .collect();
    found.sort_by_key(|(is_dir, n)| (!is_dir, n.to_lowercase()));
    found.truncate(MAX_SUGGESTIONS);
    found
        .into_iter()
        .map(|(is_dir, n)| if is_dir { format!("{n}/") } else { n })
        .collect()
}

/// The value with the typed prefix replaced by the suggestion; a folder gets the separator the
/// value already uses, ready to go one level deeper.
pub(super) fn complete(value: &str, suggestion: &str) -> String {
    let (dir, _) = split(value);
    let quote = if value.starts_with('"') { "\"" } else { "" };
    let sep = if dir.ends_with('\\') { "\\" } else { "/" };
    match suggestion.strip_suffix('/') {
        Some(name) => format!("{quote}{dir}{name}{sep}"),
        None => format!("{quote}{dir}{suggestion}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_matching_entries() {
        let root = std::env::temp_dir().join(format!("sdeck-complete-{}", std::process::id()));
        for d in ["alpha", "Alps", "beta", ".hidden"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("al.txt"), "").unwrap();
        let base = format!("{}/", root.to_string_lossy().replace('\\', "/"));
        let folders = |v: &str| path_suggestions(v, false);
        assert_eq!(folders(&format!("{base}al")), ["alpha/", "Alps/"]);
        assert_eq!(folders(&base), ["alpha/", "Alps/", "beta/"]);
        assert_eq!(folders(&format!("{base}.")), [".hidden/"]);
        assert!(folders(&format!("{base}zzz")).is_empty());
        assert_eq!(
            path_suggestions(&format!("{base}al"), true),
            ["alpha/", "Alps/", "al.txt"]
        );
        assert_eq!(complete(&format!("{base}al"), "alpha/"), format!("{base}alpha/"));
        assert_eq!(complete(&format!("{base}al"), "al.txt"), format!("{base}al.txt"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
