//! Modrinth: the site's API, its `.mrpack` format, and installing from either.
//!
//! Split three ways because those are three jobs. `api` speaks HTTP and knows
//! nothing about instances; `mrpack` reads an archive and touches no network;
//! `install` is the only part that writes to the instance.

mod api;
mod install;
mod mrpack;

pub use api::{download_file_by_sha1, list_project_files, search_projects};
pub use install::{install_modpack, install_modpack_from_file, upgrade_modpack};
pub use mrpack::read_local_modpack;
