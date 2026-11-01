//! Entity and schema designer: a column table becomes an entity (or Prisma model, Mongoose
//! schema, Sequelize model), DTOs with `class-validator` rules and Swagger decorators, and
//! the `forFeature` registration. `sync_dto` regenerates the create DTO from the entity.

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::changeset::{Changeset, Tree};
use crate::db::Orm;
use crate::edit;
use crate::generate::{dto_source, DtoField};
use crate::model::{ClassInfo, ProjectModel};
use crate::naming::{camelize, classify, normalize_name, singular};
use crate::project::{parent_dir, relative_import, ProjectMeta};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntityField {
    pub name: String,
    /// string | text | int | float | decimal | boolean | date | uuid | email | json
    pub ty: String,
    pub nullable: bool,
    pub unique: bool,
    pub length: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EntityOptions {
    pub name: String,
    /// The module folder, e.g. `src/invoices`.
    pub directory: String,
    /// Module to register the entity in.
    pub module: Option<String>,
    pub orm: Orm,
    pub fields: Vec<EntityField>,
    pub dtos: bool,
    pub swagger: bool,
    pub timestamps: bool,
}

fn ts_type(ty: &str) -> &'static str {
    match ty {
        "int" | "float" | "decimal" => "number",
        "boolean" => "boolean",
        "date" => "Date",
        "json" => "Record<string, unknown>",
        _ => "string",
    }
}

fn dto_type(ty: &str) -> &'static str {
    match ty {
        "int" => "int",
        "float" | "decimal" => "number",
        "boolean" => "boolean",
        "date" => "date",
        "uuid" => "uuid",
        "email" => "email",
        _ => "string",
    }
}

fn typeorm_column(f: &EntityField) -> String {
    let mut opts: Vec<String> = vec![];
    let ty = match f.ty.as_str() {
        "text" => Some("'text'"),
        "int" => Some("'int'"),
        "float" => Some("'float'"),
        "decimal" => {
            opts.push("precision: 12".into());
            opts.push("scale: 2".into());
            Some("'decimal'")
        }
        "json" => Some("'simple-json'"),
        _ => None,
    };
    if let Some(l) = f.length.filter(|_| matches!(f.ty.as_str(), "string" | "email")) {
        opts.push(format!("length: {l}"));
    }
    if f.nullable {
        opts.push("nullable: true".into());
    }
    if f.unique {
        opts.push("unique: true".into());
    }
    if f.ty == "boolean" && !f.nullable {
        opts.push("default: false".into());
    }
    let o = if opts.is_empty() { String::new() } else { format!("{{ {} }}", opts.join(", ")) };
    match (ty, o.is_empty()) {
        (Some(t), true) => format!("@Column({t})"),
        (Some(t), false) => format!("@Column({t}, {o})"),
        (None, true) => "@Column()".into(),
        (None, false) => format!("@Column({o})"),
    }
}

fn entity_source(class: &str, opts: &EntityOptions) -> (String, String) {
    let fields = &opts.fields;
    match opts.orm {
        Orm::TypeOrm => {
            let mut imports = vec!["Column", "Entity", "PrimaryGeneratedColumn"];
            let mut body = vec!["  @PrimaryGeneratedColumn()\n  id: number;".to_string()];
            for f in fields {
                body.push(format!("  {}\n  {}: {};", typeorm_column(f), camelize(&f.name), ts_type(&f.ty)));
            }
            if opts.timestamps {
                imports.extend(["CreateDateColumn", "UpdateDateColumn"]);
                body.push("  @CreateDateColumn()\n  createdAt: Date;".into());
                body.push("  @UpdateDateColumn()\n  updatedAt: Date;".into());
            }
            imports.sort();
            (
                "entities".into(),
                format!("import {{ {} }} from 'typeorm';\n\n@Entity()\nexport class {class} {{\n{}\n}}\n", imports.join(", "), body.join("\n\n")),
            )
        }
        Orm::MikroOrm => {
            let mut imports = vec!["Entity", "PrimaryKey", "Property"];
            let mut body = vec!["  @PrimaryKey()\n  id!: number;".to_string()];
            for f in fields {
                let mut o = vec![];
                if f.nullable {
                    o.push("nullable: true");
                }
                if f.unique {
                    o.push("unique: true");
                }
                if f.ty == "text" {
                    o.push("type: 'text'");
                }
                if f.ty == "json" {
                    o.push("type: 'json'");
                }
                let deco = if o.is_empty() { "@Property()".to_string() } else { format!("@Property({{ {} }})", o.join(", ")) };
                let bang = if f.nullable { "?" } else { "!" };
                body.push(format!("  {deco}\n  {}{bang}: {};", camelize(&f.name), ts_type(&f.ty)));
            }
            if opts.timestamps {
                body.push("  @Property()\n  createdAt = new Date();".into());
                body.push("  @Property({ onUpdate: () => new Date() })\n  updatedAt = new Date();".into());
            }
            imports.sort();
            ("entities".into(), format!("import {{ {} }} from '@mikro-orm/core';\n\n@Entity()\nexport class {class} {{\n{}\n}}\n", imports.join(", "), body.join("\n\n")))
        }
        Orm::Mongoose => {
            let mut body = vec![];
            for f in fields {
                let mut o = vec![];
                if !f.nullable {
                    o.push("required: true".to_string());
                }
                if f.unique {
                    o.push("unique: true".into());
                }
                if f.ty == "json" {
                    o.push("type: Object".into());
                }
                let deco = if o.is_empty() { "@Prop()".to_string() } else { format!("@Prop({{ {} }})", o.join(", ")) };
                body.push(format!("  {deco}\n  {}: {};", camelize(&f.name), ts_type(&f.ty)));
            }
            let schema_opts = if opts.timestamps { "{ timestamps: true }" } else { "" };
            (
                "schemas".into(),
                format!(
                    "import {{ Prop, Schema, SchemaFactory }} from '@nestjs/mongoose';\nimport {{ HydratedDocument }} from 'mongoose';\n\nexport type {class}Document = HydratedDocument<{class}>;\n\n@Schema({schema_opts})\nexport class {class} {{\n{}\n}}\n\nexport const {class}Schema = SchemaFactory.createForClass({class});\n",
                    body.join("\n\n")
                ),
            )
        }
        Orm::Sequelize => {
            let mut body = vec![];
            for f in fields {
                let dt = match f.ty.as_str() {
                    "text" => "DataType.TEXT",
                    "int" => "DataType.INTEGER",
                    "float" => "DataType.FLOAT",
                    "decimal" => "DataType.DECIMAL(12, 2)",
                    "boolean" => "DataType.BOOLEAN",
                    "date" => "DataType.DATE",
                    "uuid" => "DataType.UUID",
                    "json" => "DataType.JSON",
                    _ => "DataType.STRING",
                };
                let mut o = vec![format!("type: {dt}")];
                if !f.nullable {
                    o.push("allowNull: false".into());
                }
                if f.unique {
                    o.push("unique: true".into());
                }
                body.push(format!("  @Column({{ {} }})\n  declare {}: {};", o.join(", "), camelize(&f.name), ts_type(&f.ty)));
            }
            let table = if opts.timestamps { "@Table" } else { "@Table({ timestamps: false })" };
            ("models".into(), format!("import {{ Column, DataType, Model, Table }} from 'sequelize-typescript';\n\n{table}\nexport class {class} extends Model {{\n{}\n}}\n", body.join("\n\n")))
        }
        Orm::Prisma => (String::new(), String::new()),
    }
}

fn prisma_model(class: &str, opts: &EntityOptions, mongo: bool) -> String {
    let mut lines = vec![if mongo { "  id String @id @default(auto()) @map(\"_id\") @db.ObjectId".to_string() } else { "  id Int @id @default(autoincrement())".to_string() }];
    for f in &opts.fields {
        let ty = match f.ty.as_str() {
            "int" => "Int",
            "float" => "Float",
            "decimal" => "Decimal",
            "boolean" => "Boolean",
            "date" => "DateTime",
            "json" => "Json",
            _ => "String",
        };
        let mut attrs = String::new();
        if f.unique {
            attrs.push_str(" @unique");
        }
        if f.ty == "boolean" && !f.nullable {
            attrs.push_str(" @default(false)");
        }
        lines.push(format!("  {} {}{}{}", camelize(&f.name), ty, if f.nullable { "?" } else { "" }, attrs));
    }
    if opts.timestamps {
        lines.push("  createdAt DateTime @default(now())".into());
        lines.push("  updatedAt DateTime @updatedAt".into());
    }
    format!("\nmodel {class} {{\n{}\n}}\n", lines.join("\n"))
}

pub fn generate_entity(root: &Path, meta: &ProjectMeta, model: &ProjectModel, opts: &EntityOptions) -> Result<Changeset> {
    let name = singular(&normalize_name(opts.name.trim()));
    if name.is_empty() {
        bail!("Enter an entity name");
    }
    if opts.fields.iter().any(|f| f.name.trim().is_empty()) {
        bail!("Every column needs a name");
    }
    let class = classify(&name);
    let dir = opts.directory.trim_end_matches('/').to_string();
    let mut tree = Tree::new(root);
    let mut notes = vec![];
    let mut install = vec![];

    let module_file = opts.module.as_ref().and_then(|m| model.modules.iter().find(|x| &x.name == m)).map(|m| m.file.clone());
    if opts.orm == Orm::Prisma {
        let schema_path = "prisma/schema.prisma";
        let schema = tree.read(schema_path).ok_or_else(|| anyhow!("prisma/schema.prisma not found. Connect Prisma in the database manager first."))?;
        if schema.contains(&format!("model {class} {{")) {
            bail!("model {class} already exists in the Prisma schema");
        }
        let mongo = schema.contains("provider = \"mongodb\"");
        tree.write(schema_path, format!("{}{}", schema, prisma_model(&class, opts, mongo)));
        notes.push("Run a migration (prisma migrate dev) to create the table.".into());
    } else {
        let (folder, source) = entity_source(&class, opts);
        let suffix = match opts.orm {
            Orm::Mongoose => "schema",
            Orm::Sequelize => "model",
            _ => "entity",
        };
        let entity_path = format!("{dir}/{folder}/{name}.{suffix}.ts");
        if tree.exists(&entity_path) {
            bail!("{entity_path} already exists");
        }
        tree.write(&entity_path, source);
        if let Some(mf) = &module_file {
            let (orm_module, pkg, entry) = match opts.orm {
                Orm::Mongoose => ("MongooseModule", "@nestjs/mongoose", format!("{{ name: {class}.name, schema: {class}Schema }}")),
                Orm::Sequelize => ("SequelizeModule", "@nestjs/sequelize", class.clone()),
                Orm::MikroOrm => ("MikroOrmModule", "@mikro-orm/nestjs", class.clone()),
                _ => ("TypeOrmModule", "@nestjs/typeorm", class.clone()),
            };
            let src = tree.read(mf).unwrap_or_default();
            let mut next = match edit::add_to_for_feature(&src, orm_module, &entry) {
                Some(n) => n,
                None => {
                    let s = edit::insert_import(&src, orm_module, pkg);
                    edit::insert_metadata(&s, "imports", &format!("{orm_module}.forFeature([{entry}])")).unwrap_or(s)
                }
            };
            let path = relative_import(mf, &entity_path);
            next = edit::insert_import(&next, &class, &path);
            if opts.orm == Orm::Mongoose {
                next = edit::insert_import(&next, &format!("{class}Schema"), &path);
            }
            tree.write(mf, next);
        }
        if opts.orm == Orm::TypeOrm {
            notes.push("With synchronize on in development, TypeORM creates the table on the next start.".into());
        }
    }

    if opts.dtos {
        let fields: Vec<DtoField> = opts
            .fields
            .iter()
            .filter(|f| f.ty != "json")
            .map(|f| DtoField { name: f.name.clone(), ty: dto_type(&f.ty).into(), optional: f.nullable, array: false })
            .collect();
        let swagger = opts.swagger || meta.has_dep("@nestjs/swagger");
        let (create, pkgs) = dto_source(&format!("create-{name}"), &fields, swagger);
        let create_path = format!("{dir}/dto/create-{name}.dto.ts");
        let update_path = format!("{dir}/dto/update-{name}.dto.ts");
        if tree.exists(&create_path) {
            notes.push(format!("{create_path} exists; use Sync DTO to update it from the entity."));
        } else {
            tree.write(&create_path, create);
            let mapped = if swagger { "@nestjs/swagger" } else { "@nestjs/mapped-types" };
            if !tree.exists(&update_path) {
                tree.write(&update_path, format!("import {{ PartialType }} from '{mapped}';\nimport {{ Create{class}Dto }} from './create-{name}.dto';\n\nexport class Update{class}Dto extends PartialType(Create{class}Dto) {{}}\n"));
            }
            if !meta.has_dep(mapped) {
                install.push(mapped.to_string());
            }
            for p in pkgs {
                if !meta.has_dep(&p) {
                    install.push(p);
                }
            }
        }
    }
    let mut cs = tree.into_changeset(format!("Design entity {class}"), None);
    cs.notes = notes;
    cs.install = install;
    Ok(cs)
}

const SKIP_DECORATORS: &[&str] = &["PrimaryGeneratedColumn", "PrimaryColumn", "CreateDateColumn", "UpdateDateColumn", "DeleteDateColumn", "VersionColumn", "PrimaryKey", "ManyToOne", "OneToMany", "ManyToMany", "OneToOne"];

/// Regenerates `dto/create-<name>.dto.ts` from the entity's current columns.
pub fn sync_dto(root: &Path, meta: &ProjectMeta, entity: &ClassInfo) -> Result<Changeset> {
    let name = normalize_name(&entity.name);
    let entity_dir = parent_dir(&entity.file);
    let module_dir = if entity_dir.ends_with("/entities") || entity_dir.ends_with("/schemas") || entity_dir.ends_with("/models") { parent_dir(entity_dir) } else { entity_dir };
    let fields: Vec<DtoField> = entity
        .fields
        .iter()
        .filter(|f| !f.decorators.iter().any(|d| SKIP_DECORATORS.contains(&d.as_str())))
        .filter(|f| !matches!(f.name.as_str(), "id" | "_id" | "createdAt" | "updatedAt"))
        .filter(|f| !f.decorators.is_empty())
        .map(|f| {
            let t = f.type_text.replace(" | null", "");
            let ty = match t.trim() {
                "number" => "number",
                "boolean" => "boolean",
                "Date" => "date",
                _ => "string",
            };
            DtoField { name: f.name.clone(), ty: ty.into(), optional: f.optional || f.type_text.contains("null"), array: t.ends_with("[]") }
        })
        .collect();
    let (content, _) = dto_source(&format!("create-{name}"), &fields, meta.has_dep("@nestjs/swagger"));
    let mut tree = Tree::new(root);
    tree.write(&format!("{module_dir}/dto/create-{name}.dto.ts"), content);
    let mut cs = tree.into_changeset(format!("Sync Create{}Dto from {}", entity.name, entity.name), None);
    cs.notes.push(format!("Fields, validation rules and Swagger decorators follow {}'s columns.", entity.name));
    Ok(cs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typeorm_entity() {
        let opts = EntityOptions {
            name: "invoices".into(),
            directory: "src/invoices".into(),
            module: None,
            orm: Orm::TypeOrm,
            fields: vec![
                EntityField { name: "customer".into(), ty: "string".into(), nullable: false, unique: false, length: Some(120) },
                EntityField { name: "total".into(), ty: "decimal".into(), nullable: true, unique: false, length: None },
            ],
            dtos: true,
            swagger: true,
            timestamps: true,
        };
        let (folder, src) = entity_source("Invoice", &opts);
        assert_eq!(folder, "entities");
        assert!(src.starts_with("import { Column, CreateDateColumn, Entity, PrimaryGeneratedColumn, UpdateDateColumn } from 'typeorm';"));
        assert!(src.contains("  @Column({ length: 120 })\n  customer: string;"));
        assert!(src.contains("  @Column('decimal', { precision: 12, scale: 2, nullable: true })\n  total: number;"));
    }
}
