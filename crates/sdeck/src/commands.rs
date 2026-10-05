//! The `:` command palette's entries, port of `commands.ts`. Each wraps an existing key's action
//! under a typed name; `App` runs them by [`CommandId`].

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    NewSession,
    NewSessionChooseAgent,
    Rename,
    MoveToFolder,
    OpenConfig,
    ChooseTheme,
    ToggleSidebar,
    OpenTrash,
}

pub struct PaletteCommand {
    pub id: CommandId,
    pub label: &'static str,
}

pub const PALETTE_COMMANDS: [PaletteCommand; 8] = [
    PaletteCommand {
        id: CommandId::NewSession,
        label: "New session",
    },
    PaletteCommand {
        id: CommandId::NewSessionChooseAgent,
        label: "New session, choose agent",
    },
    PaletteCommand {
        id: CommandId::Rename,
        label: "Rename",
    },
    PaletteCommand {
        id: CommandId::MoveToFolder,
        label: "Move project to folder",
    },
    PaletteCommand {
        id: CommandId::OpenConfig,
        label: "Open config",
    },
    PaletteCommand {
        id: CommandId::ChooseTheme,
        label: "Choose theme",
    },
    PaletteCommand {
        id: CommandId::ToggleSidebar,
        label: "Toggle sidebar",
    },
    PaletteCommand {
        id: CommandId::OpenTrash,
        label: "Open trash",
    },
];

/// Every typed character of `query` must appear, in order, somewhere in a label (case- and
/// whitespace-insensitive): no scoring, no ranking. An empty query matches every label, in list
/// order. Returns the matching label indexes.
pub fn palette_matches(query: &str, labels: &[&str]) -> Vec<usize> {
    let squash = |s: &str| -> Vec<char> { s.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect() };
    let q = squash(query);
    labels
        .iter()
        .enumerate()
        .filter(|(_, label)| {
            let chars = squash(label);
            let mut pos = 0;
            q.iter().all(|c| match chars[pos..].iter().position(|x| x == c) {
                Some(found) => {
                    pos += found + 1;
                    true
                }
                None => false,
            })
        })
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_is_an_order_preserving_case_and_whitespace_insensitive_subsequence_filter() {
        let labels = ["New session", "Rename", "Move project to folder"];
        assert_eq!(palette_matches("ns", &labels), [0]);
        assert_eq!(palette_matches("NS", &labels), [0]);
        assert_eq!(palette_matches("zz", &labels), Vec::<usize>::new());
        assert_eq!(palette_matches("", &labels), [0, 1, 2]);
        assert_eq!(palette_matches("mptf", &labels), [2]);
    }

    #[test]
    fn entries_have_a_unique_id_and_a_non_empty_label() {
        assert!(!PALETTE_COMMANDS.is_empty());
        for (i, command) in PALETTE_COMMANDS.iter().enumerate() {
            assert!(!command.label.trim().is_empty());
            assert!(PALETTE_COMMANDS[..i].iter().all(|c| c.id != command.id));
        }
    }
}
