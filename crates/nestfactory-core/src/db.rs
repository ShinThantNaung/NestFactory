//! Database connection manager: plans the packages, module config, `.env` entries, Docker
//! service and validation schema for an ORM/database pair, as one changeset.

use std::path::Path;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::changeset::{Changeset, Tree};
use crate::edit;
use crate::env;
use crate::model::ProjectModel;
use crate::project::{import_spec, ProjectMeta};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Orm {
    TypeOrm,
    Prisma,
    Mongoose,
    MikroOrm,
    Sequelize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Database {
    Postgres,
    Mysql,
    Sqlite,
    Mongodb,
}

impl Database {
    pub fn default_port(self) -> u32 {
        match self {
            Database::Postgres => 5432,
            Database::Mysql => 3306,
            Database::Mongodb => 27017,
            Database::Sqlite => 0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Database::Postgres => "PostgreSQL",
            Database::Mysql => "MySQL",
            Database::Sqlite => "SQLite",
            Database::Mongodb => "MongoDB",
        }
    }
}

impl Orm {
    pub fn label(self) -> &'static str {
        match self {
            Orm::TypeOrm => "TypeORM",
            Orm::Prisma => "Prisma",
            Orm::Mongoose => "Mongoose",
            Orm::MikroOrm => "MikroORM",
            Orm::Sequelize => "Sequelize",
        }
    }

    pub fn supports(self, db: Database) -> bool {
        match self {
            Orm::Mongoose => db == Database::Mongodb,
            Orm::Sequelize => db != Database::Mongodb,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DbConfig {
    /// Connection profile name, e.g. `orders_dev`.
    pub name: String,
    pub orm: Orm,
    pub database: Database,
    pub host: String,
    pub port: u32,
    pub database_name: String,
    pub user: String,
    pub password: String,
    pub synchronize: bool,
    pub docker: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DockerService {
    pub service: String,
    pub container: String,
    pub image: String,
    pub port: u32,
    pub volume: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DbPlan {
    pub changeset: Changeset,
    /// Commands to run after installing, e.g. `prisma generate`.
    pub post_commands: Vec<Vec<String>>,
    pub env_keys: Vec<String>,
    pub docker: Option<DockerService>,
    pub module_file: String,
}

pub fn validate(cfg: &DbConfig) -> Result<()> {
    if !cfg.orm.supports(cfg.database) {
        bail!("{} doesn't support {}", cfg.orm.label(), cfg.database.label());
    }
    if cfg.database != Database::Sqlite && cfg.host.trim().is_empty() {
        bail!("Enter a host");
    }
    if cfg.database_name.trim().is_empty() {
        bail!("Enter a database name");
    }
    Ok(())
}

pub fn packages(cfg: &DbConfig) -> (Vec<String>, Vec<String>) {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match cfg.orm {
        Orm::TypeOrm => {
            let driver = match cfg.database {
                Database::Postgres => "pg",
                Database::Mysql => "mysql2",
                Database::Sqlite => "better-sqlite3",
                Database::Mongodb => "mongodb",
            };
            (s(&["@nestjs/typeorm", "typeorm", driver]), vec![])
        }
        Orm::Prisma => (s(&["@prisma/client@^6"]), s(&["prisma@^6"])),
        Orm::Mongoose => (s(&["@nestjs/mongoose", "mongoose"]), vec![]),
        Orm::Sequelize => {
            let mut p = s(&["@nestjs/sequelize", "sequelize", "sequelize-typescript"]);
            match cfg.database {
                Database::Postgres => p.extend(s(&["pg", "pg-hstore"])),
                Database::Mysql => p.push("mysql2".into()),
                _ => p.push("sqlite3".into()),
            }
            (p, vec![])
        }
        Orm::MikroOrm => {
            let driver = match cfg.database {
                Database::Postgres => "@mikro-orm/postgresql",
                Database::Mysql => "@mikro-orm/mysql",
                Database::Sqlite => "@mikro-orm/sqlite",
                Database::Mongodb => "@mikro-orm/mongodb",
            };
            (s(&["@mikro-orm/core", "@mikro-orm/nestjs", driver]), vec![])
        }
    }
}

/// `@scope/name@^1` → `@scope/name`, `name@^1` → `name`.
pub fn package_name(spec: &str) -> &str {
    let from = if spec.starts_with('@') { 1 } else { 0 };
    match spec[from..].find('@') {
        Some(i) => &spec[..i + from],
        None => spec,
    }
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn connection_url(cfg: &DbConfig) -> String {
    let auth = if cfg.user.is_empty() { String::new() } else { format!("{}:{}@", encode(&cfg.user), encode(&cfg.password)) };
    match cfg.database {
        Database::Postgres => format!("postgresql://{auth}{}:{}/{}?schema=public", cfg.host, cfg.port, cfg.database_name),
        Database::Mysql => format!("mysql://{auth}{}:{}/{}", cfg.host, cfg.port, cfg.database_name),
        Database::Mongodb => format!("mongodb://{auth}{}:{}/{}?authSource=admin", cfg.host, cfg.port, cfg.database_name),
        Database::Sqlite => format!("file:./{}", cfg.database_name),
    }
}

/// The `.env` entries this configuration needs.
pub fn env_vars(cfg: &DbConfig) -> Vec<(String, String)> {
    let kv = |k: &str, v: String| (k.to_string(), v);
    if cfg.orm == Orm::Prisma {
        return vec![kv("DATABASE_URL", connection_url(cfg))];
    }
    match cfg.database {
        Database::Mongodb => vec![kv("MONGODB_URI", connection_url(cfg))],
        Database::Sqlite => vec![kv("DB_NAME", cfg.database_name.clone())],
        _ => vec![
            kv("DB_HOST", cfg.host.clone()),
            kv("DB_PORT", cfg.port.to_string()),
            kv("DB_NAME", cfg.database_name.clone()),
            kv("DB_USER", cfg.user.clone()),
            kv("DB_PASSWORD", cfg.password.clone()),
        ],
    }
}

const FACTORY_HEAD: &str = "      imports: [ConfigModule],\n      inject: [ConfigService],\n      useFactory: (config: ConfigService) => ({\n";

fn sql_fields(user_key: &str, name_key: &str) -> String {
    format!(
        "        host: config.get<string>('DB_HOST', 'localhost'),\n        port: Number(config.get('DB_PORT')),\n        {user_key}: config.get<string>('DB_USER'),\n        password: config.get<string>('DB_PASSWORD'),\n        {name_key}: config.get<string>('DB_NAME'),\n"
    )
}

fn sync_expr(cfg: &DbConfig) -> &'static str {
    if cfg.synchronize { "config.get<string>('NODE_ENV') !== 'production'" } else { "false" }
}

fn database_module(cfg: &DbConfig) -> String {
    let (imports, body) = match cfg.orm {
        Orm::TypeOrm => {
            let fields = match cfg.database {
                Database::Postgres | Database::Mysql => {
                    let ty = if cfg.database == Database::Postgres { "postgres" } else { "mysql" };
                    format!("        type: '{ty}',\n{}", sql_fields("username", "database"))
                }
                Database::Sqlite => "        type: 'better-sqlite3',\n        database: config.get<string>('DB_NAME', 'dev.sqlite'),\n".to_string(),
                Database::Mongodb => "        type: 'mongodb',\n        url: config.get<string>('MONGODB_URI'),\n".to_string(),
            };
            (
                "import { TypeOrmModule } from '@nestjs/typeorm';",
                format!("    TypeOrmModule.forRootAsync({{\n{FACTORY_HEAD}{fields}        autoLoadEntities: true,\n        synchronize: {},\n      }}),\n    }}),", sync_expr(cfg)),
            )
        }
        Orm::Mongoose => (
            "import { MongooseModule } from '@nestjs/mongoose';",
            format!("    MongooseModule.forRootAsync({{\n{FACTORY_HEAD}        uri: config.get<string>('MONGODB_URI'),\n      }}),\n    }}),"),
        ),
        Orm::Sequelize => {
            let fields = match cfg.database {
                Database::Sqlite => "        dialect: 'sqlite',\n        storage: config.get<string>('DB_NAME', 'dev.sqlite'),\n".to_string(),
                db => {
                    let d = if db == Database::Mysql { "mysql" } else { "postgres" };
                    format!("        dialect: '{d}',\n{}", sql_fields("username", "database"))
                }
            };
            (
                "import { SequelizeModule } from '@nestjs/sequelize';",
                format!("    SequelizeModule.forRootAsync({{\n{FACTORY_HEAD}{fields}        autoLoadModels: true,\n        synchronize: {},\n      }}),\n    }}),", sync_expr(cfg)),
            )
        }
        Orm::MikroOrm => {
            let (driver, pkg) = match cfg.database {
                Database::Postgres => ("PostgreSqlDriver", "@mikro-orm/postgresql"),
                Database::Mysql => ("MySqlDriver", "@mikro-orm/mysql"),
                Database::Sqlite => ("SqliteDriver", "@mikro-orm/sqlite"),
                Database::Mongodb => ("MongoDriver", "@mikro-orm/mongodb"),
            };
            let fields = match cfg.database {
                Database::Sqlite => "        dbName: config.get<string>('DB_NAME', 'dev.sqlite'),\n".to_string(),
                Database::Mongodb => "        clientUrl: config.get<string>('MONGODB_URI'),\n".to_string(),
                _ => sql_fields("user", "dbName"),
            };
            return format!(
                "import {{ Module }} from '@nestjs/common';\nimport {{ ConfigModule, ConfigService }} from '@nestjs/config';\nimport {{ MikroOrmModule }} from '@mikro-orm/nestjs';\nimport {{ {driver} }} from '{pkg}';\n\n@Module({{\n  imports: [\n    MikroOrmModule.forRootAsync({{\n      driver: {driver},\n{FACTORY_HEAD}        driver: {driver},\n{fields}        autoLoadEntities: true,\n      }}),\n    }}),\n  ],\n}})\nexport class DatabaseModule {{}}\n"
            );
        }
        Orm::Prisma => unreachable!(),
    };
    format!(
        "import {{ Module }} from '@nestjs/common';\nimport {{ ConfigModule, ConfigService }} from '@nestjs/config';\n{imports}\n\n@Module({{\n  imports: [\n{body}\n  ],\n}})\nexport class DatabaseModule {{}}\n"
    )
}

fn prisma_schema(cfg: &DbConfig) -> String {
    let provider = match cfg.database {
        Database::Postgres => "postgresql",
        Database::Mysql => "mysql",
        Database::Sqlite => "sqlite",
        Database::Mongodb => "mongodb",
    };
    format!(
        "// Prisma schema. Models added by NestFactory's entity designer go below.\n// Docs: https://pris.ly/d/prisma-schema\n\ngenerator client {{\n  provider = \"prisma-client-js\"\n}}\n\ndatasource db {{\n  provider = \"{provider}\"\n  url      = env(\"DATABASE_URL\")\n}}\n"
    )
}

const PRISMA_SERVICE: &str = "import { Injectable, OnModuleInit } from '@nestjs/common';\nimport { PrismaClient } from '@prisma/client';\n\n@Injectable()\nexport class PrismaService extends PrismaClient implements OnModuleInit {\n  async onModuleInit() {\n    await this.$connect();\n  }\n}\n";

const PRISMA_MODULE: &str = "import { Global, Module } from '@nestjs/common';\nimport { PrismaService } from './prisma.service';\n\n@Global()\n@Module({\n  providers: [PrismaService],\n  exports: [PrismaService],\n})\nexport class PrismaModule {}\n";

pub fn docker_service(project: &str, cfg: &DbConfig) -> Option<DockerService> {
    if !cfg.docker || cfg.database == Database::Sqlite {
        return None;
    }
    let slug: String = project.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let base = slug.trim_end_matches("-api").trim_matches('-').to_string();
    let (service, image, data) = match cfg.database {
        Database::Postgres => ("postgres", "postgres:16", "pgdata"),
        Database::Mysql => ("mysql", "mysql:8.4", "mysqldata"),
        Database::Mongodb => ("mongo", "mongo:7", "mongodata"),
        Database::Sqlite => unreachable!(),
    };
    Some(DockerService {
        service: service.into(),
        container: format!("{base}-db"),
        image: image.into(),
        port: cfg.port,
        volume: Some(format!("{base}-{data}")),
    })
}

fn compose_block(d: &DockerService, cfg: &DbConfig) -> String {
    let env = match cfg.database {
        Database::Postgres => {
            if cfg.orm == Orm::Prisma {
                format!("      POSTGRES_USER: {}\n      POSTGRES_PASSWORD: {}\n      POSTGRES_DB: {}\n", cfg.user, cfg.password, cfg.database_name)
            } else {
                "      POSTGRES_USER: ${DB_USER}\n      POSTGRES_PASSWORD: ${DB_PASSWORD}\n      POSTGRES_DB: ${DB_NAME}\n".to_string()
            }
        }
        Database::Mysql => {
            let pw = if cfg.orm == Orm::Prisma { cfg.password.clone() } else { "${DB_PASSWORD}".into() };
            let name = if cfg.orm == Orm::Prisma { cfg.database_name.clone() } else { "${DB_NAME}".into() };
            if cfg.user == "root" {
                format!("      MYSQL_ROOT_PASSWORD: {pw}\n      MYSQL_DATABASE: {name}\n")
            } else {
                let user = if cfg.orm == Orm::Prisma { cfg.user.clone() } else { "${DB_USER}".into() };
                format!("      MYSQL_ROOT_PASSWORD: {pw}\n      MYSQL_DATABASE: {name}\n      MYSQL_USER: {user}\n      MYSQL_PASSWORD: {pw}\n")
            }
        }
        Database::Mongodb => format!("      MONGO_INITDB_ROOT_USERNAME: {}\n      MONGO_INITDB_ROOT_PASSWORD: {}\n", cfg.user, cfg.password),
        Database::Sqlite => String::new(),
    };
    let internal = cfg.database.default_port();
    let data_dir = match cfg.database {
        Database::Postgres => "/var/lib/postgresql/data",
        Database::Mysql => "/var/lib/mysql",
        _ => "/data/db",
    };
    let volume = d.volume.as_ref().map(|v| format!("    volumes:\n      - {v}:{data_dir}\n")).unwrap_or_default();
    format!(
        "  {}:\n    image: {}\n    container_name: {}\n    restart: unless-stopped\n    environment:\n{env}    ports:\n      - \"{}:{internal}\"\n{volume}",
        d.service, d.image, d.container, d.port
    )
}

/// Adds the service to docker-compose.yml, creating the file if needed.
fn merge_compose(existing: Option<String>, d: &DockerService, cfg: &DbConfig) -> Option<String> {
    let block = compose_block(d, cfg);
    let vol = d.volume.clone();
    match existing {
        None => {
            let mut s = format!("services:\n{block}");
            if let Some(v) = vol {
                s.push_str(&format!("\nvolumes:\n  {v}:\n"));
            }
            Some(s)
        }
        Some(text) => {
            if text.contains(&format!("container_name: {}", d.container)) {
                return None;
            }
            let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
            let services = lines.iter().position(|l| l.trim_end() == "services:");
            match services {
                Some(i) => lines.insert(i + 1, block.trim_end_matches('\n').to_string()),
                None => {
                    lines.insert(0, format!("services:\n{}", block.trim_end_matches('\n')));
                }
            }
            if let Some(v) = vol {
                match lines.iter().position(|l| l.trim_end() == "volumes:" && !l.starts_with(' ')) {
                    Some(i) => lines.insert(i + 1, format!("  {v}:")),
                    None => lines.push(format!("\nvolumes:\n  {v}:")),
                }
            }
            Some(lines.join("\n") + "\n")
        }
    }
}

pub fn plan(root: &Path, meta: &ProjectMeta, model: &ProjectModel, cfg: &DbConfig) -> Result<DbPlan> {
    validate(cfg)?;
    let mut tree = Tree::with_esm(root, meta.esm);
    let mut notes = vec![];
    let sr = &meta.source_root;
    let root_module = model
        .root_modules
        .first()
        .and_then(|r| model.modules.iter().find(|m| &m.name == r))
        .map(|m| m.file.clone())
        .unwrap_or_else(|| format!("{sr}/app.module.ts"));
    let mut post_commands = vec![];
    let (mut install, install_dev) = packages(cfg);
    let module_file;

    if cfg.orm == Orm::Prisma {
        if !tree.exists("prisma/schema.prisma") {
            tree.write("prisma/schema.prisma", prisma_schema(cfg));
        } else {
            notes.push("prisma/schema.prisma already exists and is left as is.".into());
        }
        module_file = format!("{sr}/prisma/prisma.module.ts");
        if !tree.exists(&module_file) {
            tree.write(&format!("{sr}/prisma/prisma.service.ts"), PRISMA_SERVICE);
            tree.write(&module_file, PRISMA_MODULE);
        }
        if let Some(src) = tree.read(&root_module) {
            let path = import_spec(&root_module, &module_file, meta.esm);
            tree.write(&root_module, edit::declare(&src, "imports", "PrismaModule", &path));
        }
        post_commands.push(crate::pm::exec(meta.package_manager, "prisma", &["generate"]));
    } else {
        module_file = format!("{sr}/database/database.module.ts");
        let content = database_module(cfg);
        if tree.exists(&module_file) {
            notes.push(format!("Replaces the existing {module_file}."));
        }
        tree.write(&module_file, content);
        if let Some(src) = tree.read(&root_module) {
            let path = import_spec(&root_module, &module_file, meta.esm);
            let mut next = edit::declare(&src, "imports", "DatabaseModule", &path);
            if !model.modules.iter().any(|m| m.imports.iter().any(|i| i.name == "ConfigModule" && i.global)) {
                next = edit::insert_import(&next, "ConfigModule", "@nestjs/config");
                next = edit::insert_metadata(&next, "imports", "ConfigModule.forRoot({ isGlobal: true })").unwrap_or(next);
                notes.push("Adds ConfigModule.forRoot({ isGlobal: true }) so .env is loaded.".into());
            }
            tree.write(&root_module, next);
        }
        if !meta.has_dep("@nestjs/config") {
            install.push("@nestjs/config".into());
        }
    }

    // .env, .gitignore, validation schema.
    let vars = env_vars(cfg);
    let current = tree.read(".env").unwrap_or_default();
    let updates: Vec<(String, Option<String>)> = vars.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect();
    tree.write(".env", env::update_env(&current, &updates));
    let example: Vec<(String, Option<String>)> = vars
        .iter()
        .map(|(k, v)| {
            let safe = if k.contains("PASSWORD") { "change-me".to_string() } else if k.ends_with("URL") || k.ends_with("URI") { v.replace(&encode(&cfg.password), "change-me") } else { v.clone() };
            (k.clone(), Some(safe))
        })
        .collect();
    if tree.exists(".env.example") {
        let cur = tree.read(".env.example").unwrap_or_default();
        tree.write(".env.example", env::update_env(&cur, &example));
    }
    let gitignore = tree.read(".gitignore").unwrap_or_default();
    if !gitignore.lines().any(|l| l.trim() == ".env" || l.trim() == "/.env" || l.trim() == ".env*") {
        let sep = if gitignore.is_empty() || gitignore.ends_with('\n') { "" } else { "\n" };
        tree.write(".gitignore", format!("{gitignore}{sep}\n# local environment\n.env\n"));
        notes.push("Adds .env to .gitignore.".into());
    }
    let validation = format!("{sr}/config/env.validation.ts");
    if let Some(src) = tree.read(&validation) {
        let mut next = src.clone();
        for (k, _) in &vars {
            let ty = if k == "DB_PORT" { "number" } else { "string" };
            next = env::add_schema_property(&next, k, ty);
        }
        if next != src {
            tree.write(&validation, next);
        }
    }

    let docker = docker_service(&meta.name, cfg);
    if let Some(d) = &docker {
        if let Some(next) = merge_compose(tree.read("docker-compose.yml"), d, cfg) {
            tree.write("docker-compose.yml", next);
        }
    }

    let label = format!("Connect {} with {}", cfg.database.label(), cfg.orm.label());
    let mut cs = tree.into_changeset(label, None);
    cs.notes = notes;
    cs.install = install.into_iter().filter(|p| !meta.has_dep(package_name(p))).collect();
    cs.install_dev = install_dev.into_iter().filter(|p| !meta.has_dep(package_name(p))).collect();
    Ok(DbPlan { changeset: cs, post_commands, env_keys: vars.into_iter().map(|(k, _)| k).collect(), docker, module_file })
}

/// A Node script that connects with the project's own driver and prints JSON. Connection
/// details are passed through environment variables, never on the command line.
pub fn driver_test_script(cfg: &DbConfig) -> (String, Vec<(String, String)>) {
    let mut env = vec![
        ("NF_HOST".to_string(), cfg.host.clone()),
        ("NF_PORT".to_string(), cfg.port.to_string()),
        ("NF_USER".to_string(), cfg.user.clone()),
        ("NF_PASSWORD".to_string(), cfg.password.clone()),
        ("NF_DB".to_string(), cfg.database_name.clone()),
    ];
    let script = match cfg.database {
        Database::Postgres => "const {Client}=require('pg');const c=new Client({host:process.env.NF_HOST,port:+process.env.NF_PORT,user:process.env.NF_USER,password:process.env.NF_PASSWORD,database:process.env.NF_DB,connectionTimeoutMillis:5000});c.connect().then(()=>c.query('select version() as v')).then(r=>{console.log(JSON.stringify({ok:true,version:r.rows[0].v}));return c.end()}).catch(e=>{console.log(JSON.stringify({ok:false,error:e.message}));process.exit(0)});",
        Database::Mysql => "require('mysql2/promise').createConnection({host:process.env.NF_HOST,port:+process.env.NF_PORT,user:process.env.NF_USER,password:process.env.NF_PASSWORD,database:process.env.NF_DB,connectTimeout:5000}).then(async c=>{const [r]=await c.query('select version() as v');console.log(JSON.stringify({ok:true,version:'MySQL '+r[0].v}));await c.end()}).catch(e=>{console.log(JSON.stringify({ok:false,error:e.message}))});",
        Database::Mongodb => {
            env.push(("NF_URL".into(), connection_url(cfg)));
            "let m;try{m=require('mongodb')}catch{m=require('mongoose').mongo}const c=new m.MongoClient(process.env.NF_URL,{serverSelectionTimeoutMS:5000});c.connect().then(()=>c.db().admin().serverInfo()).then(i=>{console.log(JSON.stringify({ok:true,version:'MongoDB '+i.version}));return c.close()}).catch(e=>{console.log(JSON.stringify({ok:false,error:e.message}));process.exit(0)});"
        }
        Database::Sqlite => "const fs=require('fs');const p=require('path').resolve(process.env.NF_DB);try{fs.closeSync(fs.openSync(p,'a'));console.log(JSON.stringify({ok:true,version:'SQLite file '+p}))}catch(e){console.log(JSON.stringify({ok:false,error:e.message}))}",
    };
    (script.to_string(), env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(orm: Orm, db: Database) -> DbConfig {
        DbConfig {
            name: "orders_dev".into(),
            orm,
            database: db,
            host: "localhost".into(),
            port: db.default_port(),
            database_name: "orders_dev".into(),
            user: "postgres".into(),
            password: "p@ss word".into(),
            synchronize: true,
            docker: true,
        }
    }

    #[test]
    fn urls_are_encoded() {
        assert_eq!(connection_url(&cfg(Orm::Prisma, Database::Postgres)), "postgresql://postgres:p%40ss%20word@localhost:5432/orders_dev?schema=public");
    }

    #[test]
    fn typeorm_module_is_well_formed() {
        let m = database_module(&cfg(Orm::TypeOrm, Database::Postgres));
        assert!(m.contains("TypeOrmModule.forRootAsync({\n      imports: [ConfigModule],"), "{m}");
        assert!(m.contains("        type: 'postgres',\n"));
        assert!(m.ends_with("      }),\n    }),\n  ],\n})\nexport class DatabaseModule {}\n"), "{m}");
        let open = m.matches('{').count();
        let close = m.matches('}').count();
        assert_eq!(open, close, "{m}");
    }

    #[test]
    fn compose_merges() {
        let c = cfg(Orm::TypeOrm, Database::Postgres);
        let d = docker_service("orders-api", &c).unwrap();
        assert_eq!(d.container, "orders-db");
        let fresh = merge_compose(None, &d, &c).unwrap();
        assert!(fresh.starts_with("services:\n  postgres:\n    image: postgres:16\n"));
        assert!(fresh.contains("volumes:\n  orders-pgdata:\n"));
        assert!(merge_compose(Some(fresh), &d, &c).is_none());
    }
}
