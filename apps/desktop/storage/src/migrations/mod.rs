//! Migration scaffolding.
//!
//! `0001_initial.sql` is the schema baseline applied via idempotent
//! statements in [`super::db`]. Subsequent changes that touch JSON shape
//! rather than tables live in [`run`] as one-shot, idempotent data
//! migrations.

pub mod run;

pub use run::{default_projects_root, projectless_dir, run_data_migrations, DEFAULT_PROJECTS_ROOT};
