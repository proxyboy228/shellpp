//! The SFTP tab (UI 5; night plan, stage 4). So far the logic of a file
//! panel without its view: the rows of a folder and how they are sorted,
//! filtered and navigated ([`listing`]), and the local file system
//! ([`local`]); the panels themselves come next. The transfers run in
//! `shellpp_ssh::{sftp, transfer, transfer_queue}`.

pub mod listing;
pub mod local;
