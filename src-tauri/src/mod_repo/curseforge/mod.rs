//! CurseForge: the site's API, the manifest its modpack zips carry, and
//! installing from either.
//!
//! Split the same three ways as `modrinth`, by what each part touches: `api`
//! speaks HTTP, `manifest` reads an archive, `install` writes to the instance.

mod api;
mod install;
mod manifest;

pub use api::{list_project_files, project_file_page_url, search_projects};
pub use install::{install_modpack, install_modpack_from_file, upgrade_modpack};
pub use manifest::read_local_modpack;
