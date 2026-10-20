//! Readers of the import sources (SPEC "Import"; UI 2.5): the parsers that
//! do not need the vault. The service layer (`crate::service`) turns what
//! they read into records.

pub mod ssh_config;
