//! Writes a generator's output into a project, for comparing against `nest g`.
//! Usage: cargo run -p nestfactory-core --example gen -- <root> <schematic> <name> [dir] [--no-spec] [--flat]

use std::path::PathBuf;

use nestfactory_core::changeset::apply;
use nestfactory_core::generate::{generate, GenerateOptions, Schematic, Transport};
use nestfactory_core::project::read_meta;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(&args[0]);
    let schematic: Schematic = serde_json::from_value(serde_json::Value::String(args[1].clone()))?;
    let directory = args.get(3).filter(|a| !a.starts_with("--")).cloned();
    let transport = match args.iter().find_map(|a| a.strip_prefix("--type=")) {
        Some(t) => serde_json::from_value(serde_json::Value::String(t.to_string()))?,
        None => Transport::Rest,
    };
    let opts = GenerateOptions {
        schematic,
        name: args[2].clone(),
        directory,
        transport,
        crud: !args.iter().any(|a| a == "--no-crud"),
        spec: !args.iter().any(|a| a == "--no-spec"),
        flat: args.iter().any(|a| a == "--flat"),
        swagger: false,
        skip_import: false,
        fields: vec![],
    };
    let cs = generate(&root, &read_meta(&root), &opts)?;
    println!("{}", cs.command.clone().unwrap_or_default());
    apply(&root, &cs)?;
    Ok(())
}
