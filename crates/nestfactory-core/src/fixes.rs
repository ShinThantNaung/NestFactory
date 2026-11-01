//! Turns graph quick fixes (and drag-to-wire gestures) into changesets.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::changeset::{Changeset, Tree};
use crate::edit;
use crate::model::{FixAction, ProjectModel};
use crate::project::relative_import;

fn module_file<'a>(model: &'a ProjectModel, name: &str) -> Result<&'a str> {
    model.modules.iter().find(|m| m.name == name).map(|m| m.file.as_str()).ok_or_else(|| anyhow!("{name} not found"))
}

fn class_file<'a>(model: &'a ProjectModel, name: &str) -> Result<&'a str> {
    model.classes.iter().find(|c| c.name == name).map(|c| c.file.as_str()).ok_or_else(|| anyhow!("{name} not found"))
}

fn orm_package(orm_module: &str) -> &'static str {
    match orm_module {
        "MongooseModule" => "@nestjs/mongoose",
        "SequelizeModule" => "@nestjs/sequelize",
        "MikroOrmModule" => "@mikro-orm/nestjs",
        _ => "@nestjs/typeorm",
    }
}

fn import_path(model: &ProjectModel, from: &str, to: &str) -> String {
    let p = relative_import(from, to);
    if model.esm { format!("{p}.js") } else { p }
}

pub fn preview_fix(root: &Path, model: &ProjectModel, action: &FixAction) -> Result<Changeset> {
    let mut tree = Tree::with_esm(root, model.esm);
    let mut notes = vec![];
    let label;
    match action {
        FixAction::ImportModule { module, into, package } => {
            let into_file = module_file(model, into)?;
            let path = match package {
                Some(p) => p.clone(),
                None => import_path(model, into_file, module_file(model, module)?),
            };
            let src = tree.read(into_file).unwrap_or_default();
            tree.write(into_file, edit::declare(&src, "imports", module, &path));
            label = format!("Import {module} into {into}");
            notes.push(format!("{module} is added once to {into}'s imports."));
        }
        FixAction::AddProvider { provider, module } => {
            let file = module_file(model, module)?;
            let path = import_path(model, file, class_file(model, provider)?);
            let src = tree.read(file).unwrap_or_default();
            tree.write(file, edit::declare(&src, "providers", provider, &path));
            label = format!("Add {provider} to {module}");
        }
        FixAction::AddController { controller, module } => {
            let file = module_file(model, module)?;
            let path = import_path(model, file, class_file(model, controller)?);
            let src = tree.read(file).unwrap_or_default();
            tree.write(file, edit::declare(&src, "controllers", controller, &path));
            label = format!("Add {controller} to {module}");
        }
        FixAction::ExportAndImport { provider, from, into } => {
            let from_file = module_file(model, from)?;
            let src = tree.read(from_file).unwrap_or_default();
            if let Some(next) = edit::insert_metadata(&src, "exports", provider) {
                tree.write(from_file, next);
            }
            let into_file = module_file(model, into)?;
            let path = import_path(model, into_file, from_file);
            let src = tree.read(into_file).unwrap_or_default();
            tree.write(into_file, edit::declare(&src, "imports", from, &path));
            label = format!("Export {provider} from {from} and import it into {into}");
        }
        FixAction::ApplyForwardRef { modules } => {
            for m in modules {
                let info = model.modules.iter().find(|x| &x.name == m).ok_or_else(|| anyhow!("{m} not found"))?;
                let names: Vec<String> = info
                    .imports
                    .iter()
                    .filter(|i| !i.external && !i.forward_ref && modules.contains(&i.name))
                    .map(|i| i.name.clone())
                    .collect();
                if names.is_empty() {
                    continue;
                }
                let src = tree.read(&info.file).unwrap_or_default();
                tree.write(&info.file, edit::wrap_forward_refs(&src, &names));
            }
            label = format!("Apply forwardRef between {}", modules.join(" and "));
            notes.push("Providers that inject across the cycle also need @Inject(forwardRef(() => X)).".into());
        }
        FixAction::AddFeature { orm_module, entity, module } => {
            let file = module_file(model, module)?;
            let entity_file = class_file(model, entity).ok().map(str::to_string);
            let mongoose = orm_module == "MongooseModule";
            let entry = if mongoose { format!("{{ name: {entity}.name, schema: {entity}Schema }}") } else { entity.clone() };
            let src = tree.read(file).unwrap_or_default();
            let mut next = match edit::add_to_for_feature(&src, orm_module, &entry) {
                Some(n) => n,
                None => {
                    let with_import = edit::insert_import(&src, orm_module, orm_package(orm_module));
                    edit::insert_metadata(&with_import, "imports", &format!("{orm_module}.forFeature([{entry}])")).unwrap_or(with_import)
                }
            };
            if let Some(ef) = &entity_file {
                let path = import_path(model, file, ef);
                next = edit::insert_import(&next, entity, &path);
                if mongoose {
                    next = edit::insert_import(&next, &format!("{entity}Schema"), &path);
                }
            }
            tree.write(file, next);
            label = format!("Register {entity} in {module}");
        }
    }
    let mut cs = tree.into_changeset(label, None);
    cs.notes = notes;
    Ok(cs)
}

/// What dropping `provider` onto `module` in the graph should do.
pub fn wire_action(model: &ProjectModel, provider: &str, module: &str) -> Result<FixAction> {
    let owner = model.modules.iter().find(|m| m.name != module && m.providers.iter().any(|p| p.token == provider));
    if model.modules.iter().any(|m| m.name == module && m.providers.iter().any(|p| p.token == provider)) {
        return Err(anyhow!("{provider} is already a provider of {module}"));
    }
    Ok(match owner {
        Some(o) if o.exports.iter().any(|e| e == provider) => FixAction::ImportModule { module: o.name.clone(), into: module.to_string(), package: None },
        Some(o) => FixAction::ExportAndImport { provider: provider.to_string(), from: o.name.clone(), into: module.to_string() },
        None => FixAction::AddProvider { provider: provider.to_string(), module: module.to_string() },
    })
}
