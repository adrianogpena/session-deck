//! The agent registry behind `tools.<agent>.*`: Claude and Copilot have rich adapters; everything
//! else is "basic" tier — spawned in a PTY on PATH, liveness-only status.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogAgent {
    pub id: &'static str,
    /// Shown in pickers and flash messages.
    pub name: &'static str,
}

pub const BUILTIN_AGENTS: [&str; 2] = ["claude", "copilot"];

pub const CATALOG_AGENTS: [CatalogAgent; 2] = [
    CatalogAgent {
        id: "codex",
        name: "Codex",
    },
    CatalogAgent {
        id: "gemini",
        name: "Gemini",
    },
];

pub fn is_builtin_agent(id: &str) -> bool {
    BUILTIN_AGENTS.contains(&id)
}

pub fn find_catalog_agent(id: &str) -> Option<&'static CatalogAgent> {
    CATALOG_AGENTS.iter().find(|a| a.id == id)
}

/// The two rich-adapter ids plus every basic-tier catalog id.
pub fn all_agent_ids() -> Vec<&'static str> {
    BUILTIN_AGENTS
        .iter()
        .copied()
        .chain(CATALOG_AGENTS.iter().map(|a| a.id))
        .collect()
}

/// `Claude`/`Copilot` for the built-ins, a catalog entry's own name, or the bare id as a last resort.
pub fn agent_display_name(id: &str) -> String {
    match id {
        "claude" => "Claude".to_string(),
        "copilot" => "Copilot".to_string(),
        _ => find_catalog_agent(id).map_or_else(|| id.to_string(), |a| a.name.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_builtin_agent_is_true_only_for_claude_and_copilot() {
        assert!(is_builtin_agent("claude"));
        assert!(is_builtin_agent("copilot"));
        assert!(!is_builtin_agent("codex"));
        assert!(!is_builtin_agent("bogus"));
    }

    #[test]
    fn all_agent_ids_lists_builtins_then_catalog() {
        let mut expected = vec!["claude", "copilot"];
        expected.extend(CATALOG_AGENTS.iter().map(|a| a.id));
        assert_eq!(all_agent_ids(), expected);
    }

    #[test]
    fn find_catalog_agent_finds_seeded_entry_and_misses_unknown() {
        assert_eq!(find_catalog_agent("codex").map(|a| a.name), Some("Codex"));
        assert!(find_catalog_agent("bogus").is_none());
    }

    #[test]
    fn agent_display_name_falls_back_to_the_bare_id() {
        assert_eq!(agent_display_name("claude"), "Claude");
        assert_eq!(agent_display_name("copilot"), "Copilot");
        assert_eq!(agent_display_name("codex"), "Codex");
        assert_eq!(agent_display_name("some-future-agent"), "some-future-agent");
    }
}
