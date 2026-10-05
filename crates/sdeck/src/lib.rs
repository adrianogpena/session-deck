//! The sdeck terminal UI, port of `packages/cli`. A library so the binary stays a thin shell and
//! every module is testable without a real terminal.

pub mod agents;
pub mod ansi;
pub mod app;
pub mod commands;
pub mod config_fields;
pub mod ctl_client;
pub mod event;
pub mod filters;
pub mod frontmatter;
pub mod git_status_tracker;
pub mod keys;
pub mod layout;
pub mod live_session;
pub mod screen_status;
pub mod sessions;
pub mod skills;
pub mod term_widget;
pub mod theme;
pub mod tree;
pub mod view;

#[cfg(test)]
mod test_support;
