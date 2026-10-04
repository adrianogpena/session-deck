//! The sdeck terminal UI, port of `packages/cli`. A library so the binary stays a thin shell and
//! every module is testable without a real terminal.

pub mod ansi;
pub mod app;
pub mod event;
pub mod filters;
pub mod keys;
pub mod layout;
pub mod theme;
pub mod view;
