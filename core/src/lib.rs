//! canopy-core: an open-source Google Drive sync engine for Linux.
//!
//! The design follows what Insync does (see DESIGN.md):
//! - `mirror`: a local copy of the cloud's metadata, kept current with the
//!   Drive changes feed (one token per drive).
//! - `db`: the last state both sides agreed on ("base"), per root.
//! - `engine`: compares local, cloud and base three ways for each path and
//!   plans uploads, downloads, moves, deletions and conflicts.
//! - `cloud`: the Drive operations the engine needs, as a trait (`drive`
//!   is Google's REST API; `fake` is an in-memory Drive for tests).
//!
//! The daemon (`canopyd`) runs one engine per sync root, watches the disk
//! with inotify, polls the changes feed, serves a unix socket (the window,
//! the `canopy` command and grove's badges talk to it) and shows the tray
//! icon.

pub mod api;
pub mod auth;
pub mod cloud;
pub mod config;
pub mod daemon;
pub mod db;
pub mod drive;
pub mod engine;
pub mod fake;
pub mod gdocs;
pub mod handlers;
pub mod hearth;
pub mod ignore;
pub mod local;
pub mod mirror;
pub mod paths;
pub mod throttle;
pub mod tray;
pub mod trash;
pub mod watch;
