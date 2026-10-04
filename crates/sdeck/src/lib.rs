//! The sdeck terminal UI, port of `packages/cli`. A library so the binary stays a thin shell and
//! every module is testable without a real terminal.

pub mod ansi;
pub mod app;
pub mod event;
pub mod filters;
pub mod git_status_tracker;
pub mod keys;
pub mod layout;
pub mod live_session;
pub mod screen_status;
pub mod sessions;
pub mod term_widget;
pub mod theme;
pub mod tree;
pub mod view;

#[cfg(test)]
mod test_support;
