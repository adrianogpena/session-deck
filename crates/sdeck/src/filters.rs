//! Port of `filters.ts`: what the filter pills count and toggle.

pub use sdeck_core::store::tree_prefs::SessionCategory as StatusCategory;

pub const STATUS_CATEGORIES: [StatusCategory; 5] = [
    StatusCategory::Running,
    StatusCategory::Waiting,
    StatusCategory::Idle,
    StatusCategory::Error,
    StatusCategory::Stopped,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeFilter {
    #[default]
    All,
    Today,
    ThreeDays,
    SevenDays,
}

impl TimeFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all time",
            Self::Today => "today",
            Self::ThreeDays => "3 days",
            Self::SevenDays => "7 days",
        }
    }
}

/// Session counts per status category, for the header logo and the pills.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts([usize; 5]);

impl StatusCounts {
    pub fn get(&self, category: StatusCategory) -> usize {
        self.0[Self::index(category)]
    }

    pub fn set(&mut self, category: StatusCategory, count: usize) {
        self.0[Self::index(category)] = count;
    }

    fn index(category: StatusCategory) -> usize {
        STATUS_CATEGORIES.iter().position(|&c| c == category).unwrap()
    }
}
