//! NestFactory core: Nest-aware analysis and generation.
//!
//! - [`scanner`] walks a project and parses TypeScript with the in-house [`ts`] parser.
//! - [`analyze`] builds the [`model::ProjectModel`]: modules, providers, injection, routes,
//!   the module graph and diagnostics such as missing providers and circular imports.
//! - [`generate`] renders the official Nest schematic templates and wires the result into
//!   the nearest module, producing a [`changeset::Changeset`] that is previewed as a diff
//!   and applied or undone as one step.

pub mod ts {
    pub mod lexer;
    pub mod parse;
}

pub mod analyze;
pub mod changeset;
pub mod db;
pub mod edit;
pub mod entity;
pub mod env;
pub mod fixes;
pub mod generate;
pub mod layout;
pub mod model;
pub mod naming;
pub mod pm;
pub mod project;
pub mod scanner;
pub mod template;
pub mod testing;
pub mod wiring;
pub mod wizard;

pub use model::*;
pub use scanner::Scanner;
