/// Only the status enum so far (needed by `DeckConfig.ui.notifyStatuses`); the rest of
/// `sessionStatus.ts` is ported in the batch that reads and writes the status files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Waiting,
    Done,
    Error,
}

impl SessionStatus {
    pub const ALL: [SessionStatus; 4] = [Self::Running, Self::Waiting, Self::Done, Self::Error];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}
