//! Wiring a package into the root module when it's installed (`ScheduleModule.forRoot()` in
//! `AppModule`), optionally with an example file, and taking the wiring out on uninstall.
//! Both are changesets, so they're previewed as a diff and undone with Ctrl Z.

use std::path::Path;

use anyhow::{bail, Result};
use serde::Serialize;
use ts_rs::TS;

use crate::changeset::{Changeset, Tree};
use crate::edit;
use crate::model::ProjectModel;
use crate::project::relative_import;

struct Recipe {
    package: &'static str,
    /// What the step says, with `{module}` for the root module's name.
    label: &'static str,
    /// `(symbol, from)` pairs to import into the root module.
    imports: &'static [(&'static str, &'static str)],
    /// `(metadata, element, key)`: `element` goes into `@Module({ metadata })`; `key` finds it again.
    entries: &'static [(&'static str, &'static str, &'static str)],
    example: Option<Example>,
}

struct Example {
    /// Under the source root.
    path: &'static str,
    class: &'static str,
    metadata: &'static str,
    label: &'static str,
    content: &'static str,
}

const TASKS_SERVICE: &str = "import { Injectable, Logger } from '@nestjs/common';
import { Cron, CronExpression } from '@nestjs/schedule';

@Injectable()
export class TasksService {
  private readonly logger = new Logger(TasksService.name);

  @Cron(CronExpression.EVERY_MINUTE)
  handleCron() {
    this.logger.debug('Runs every minute');
  }
}
";

const HEALTH_CONTROLLER: &str = "import { Controller, Get } from '@nestjs/common';
import { HealthCheck, HealthCheckService, MemoryHealthIndicator } from '@nestjs/terminus';

@Controller('health')
export class HealthController {
  constructor(
    private readonly health: HealthCheckService,
    private readonly memory: MemoryHealthIndicator,
  ) {}

  @Get()
  @HealthCheck()
  check() {
    return this.health.check([() => this.memory.checkHeap('memory_heap', 300 * 1024 * 1024)]);
  }
}
";

const RECIPES: &[Recipe] = &[
    Recipe {
        package: "@nestjs/config",
        label: "Add `ConfigModule.forRoot({ isGlobal: true })` to `{module}` imports",
        imports: &[("ConfigModule", "@nestjs/config")],
        entries: &[("imports", "ConfigModule.forRoot({ isGlobal: true })", "ConfigModule")],
        example: None,
    },
    Recipe {
        package: "@nestjs/schedule",
        label: "Add `ScheduleModule.forRoot()` to `{module}` imports",
        imports: &[("ScheduleModule", "@nestjs/schedule")],
        entries: &[("imports", "ScheduleModule.forRoot()", "ScheduleModule")],
        example: Some(Example { path: "tasks/tasks.service.ts", class: "TasksService", metadata: "providers", label: "Generate an example `tasks.service.ts` with one `@Cron()` job", content: TASKS_SERVICE }),
    },
    Recipe {
        package: "@nestjs/event-emitter",
        label: "Add `EventEmitterModule.forRoot()` to `{module}` imports",
        imports: &[("EventEmitterModule", "@nestjs/event-emitter")],
        entries: &[("imports", "EventEmitterModule.forRoot()", "EventEmitterModule")],
        example: None,
    },
    Recipe {
        package: "@nestjs/throttler",
        label: "Add `ThrottlerModule.forRoot()` (100 requests a minute) and a global `ThrottlerGuard` to `{module}`",
        imports: &[("ThrottlerModule", "@nestjs/throttler"), ("ThrottlerGuard", "@nestjs/throttler"), ("APP_GUARD", "@nestjs/core")],
        entries: &[("imports", "ThrottlerModule.forRoot([{ ttl: 60000, limit: 100 }])", "ThrottlerModule"), ("providers", "{ provide: APP_GUARD, useClass: ThrottlerGuard }", "ThrottlerGuard")],
        example: None,
    },
    Recipe {
        package: "@nestjs/cache-manager",
        label: "Add `CacheModule.register({ isGlobal: true })` to `{module}` imports",
        imports: &[("CacheModule", "@nestjs/cache-manager")],
        entries: &[("imports", "CacheModule.register({ isGlobal: true })", "CacheModule")],
        example: None,
    },
    Recipe {
        package: "@nestjs/cqrs",
        label: "Add `CqrsModule.forRoot()` to `{module}` imports",
        imports: &[("CqrsModule", "@nestjs/cqrs")],
        entries: &[("imports", "CqrsModule.forRoot()", "CqrsModule")],
        example: None,
    },
    Recipe {
        package: "@nestjs/axios",
        label: "Add `HttpModule` to `{module}` imports",
        imports: &[("HttpModule", "@nestjs/axios")],
        entries: &[("imports", "HttpModule", "HttpModule")],
        example: None,
    },
    Recipe {
        package: "@nestjs/terminus",
        label: "Add `TerminusModule` to `{module}` imports",
        imports: &[("TerminusModule", "@nestjs/terminus")],
        entries: &[("imports", "TerminusModule", "TerminusModule")],
        example: Some(Example { path: "health/health.controller.ts", class: "HealthController", metadata: "controllers", label: "Generate `health.controller.ts` serving `GET /health`", content: HEALTH_CONTROLLER }),
    },
    Recipe {
        package: "@nestjs/jwt",
        label: "Add a global `JwtModule` signing with `JWT_SECRET` to `{module}` imports",
        imports: &[("JwtModule", "@nestjs/jwt")],
        entries: &[("imports", "JwtModule.register({ global: true, secret: process.env.JWT_SECRET, signOptions: { expiresIn: '1h' } })", "JwtModule")],
        example: None,
    },
    Recipe {
        package: "@nestjs/passport",
        label: "Add `PassportModule` to `{module}` imports",
        imports: &[("PassportModule", "@nestjs/passport")],
        entries: &[("imports", "PassportModule", "PassportModule")],
        example: None,
    },
    Recipe {
        package: "@nestjs/bullmq",
        label: "Add `BullModule.forRoot()` with a Redis connection on localhost:6379 to `{module}` imports",
        imports: &[("BullModule", "@nestjs/bullmq")],
        entries: &[("imports", "BullModule.forRoot({ connection: { host: 'localhost', port: 6379 } })", "BullModule")],
        example: None,
    },
    Recipe {
        package: "@nestjs/graphql",
        label: "Add a code-first `GraphQLModule` with the Apollo driver to `{module}` imports",
        imports: &[("GraphQLModule", "@nestjs/graphql"), ("ApolloDriver", "@nestjs/apollo"), ("ApolloDriverConfig", "@nestjs/apollo")],
        entries: &[("imports", "GraphQLModule.forRoot<ApolloDriverConfig>({ driver: ApolloDriver, autoSchemaFile: true })", "GraphQLModule")],
        example: None,
    },
    Recipe {
        package: "class-validator",
        label: "Validate every request with a global `ValidationPipe` in `{module}`",
        imports: &[("APP_PIPE", "@nestjs/core"), ("ValidationPipe", "@nestjs/common")],
        entries: &[("providers", "{ provide: APP_PIPE, useValue: new ValidationPipe({ whitelist: true, transform: true }) }", "ValidationPipe")],
        example: None,
    },
    Recipe {
        package: "nestjs-pino",
        label: "Add `LoggerModule.forRoot()` to `{module}` imports",
        imports: &[("LoggerModule", "nestjs-pino")],
        entries: &[("imports", "LoggerModule.forRoot()", "LoggerModule")],
        example: None,
    },
];

/// One optional step offered after an install.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WireStep {
    /// `module` or `example`.
    pub id: String,
    /// Code is in backticks.
    pub label: String,
    /// Already in the project; nothing to do.
    pub done: bool,
    /// The files the step edits or creates.
    pub files: Vec<String>,
}

fn recipe(package: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.package == package)
}

fn root_module(model: &ProjectModel) -> (String, String) {
    model
        .root_modules
        .first()
        .and_then(|r| model.modules.iter().find(|m| &m.name == r))
        .map(|m| (m.name.clone(), m.file.clone()))
        .unwrap_or_else(|| ("AppModule".into(), format!("{}/app.module.ts", model.source_root)))
}

fn main_file(model: &ProjectModel) -> Option<&str> {
    model.files.iter().find(|f| f.path.ends_with("main.ts")).map(|f| f.path.as_str())
}

/// Whether the recipe's main entry is already there, in the root module or main.ts
/// (`app.useGlobalPipes(new ValidationPipe())` counts for class-validator).
fn is_wired(root: &Path, model: &ProjectModel, r: &Recipe) -> bool {
    let (_, file) = root_module(model);
    let key = r.entries[0].2;
    let read = |rel: &str| std::fs::read_to_string(root.join(rel)).unwrap_or_default();
    edit::has_import(&read(&file), key) || main_file(model).is_some_and(|m| read(m).contains(key))
}

pub fn steps(root: &Path, model: &ProjectModel, package: &str) -> Vec<WireStep> {
    let Some(r) = recipe(package) else { return vec![] };
    let (name, file) = root_module(model);
    let mut out = vec![WireStep { id: "module".into(), label: r.label.replace("{module}", &name), done: is_wired(root, model, r), files: vec![file.clone()] }];
    if let Some(ex) = &r.example {
        let path = format!("{}/{}", model.source_root, ex.path);
        out.push(WireStep { id: "example".into(), label: ex.label.into(), done: root.join(&path).exists(), files: vec![path, file] });
    }
    out
}

/// The changeset for the chosen steps (`module`, `example`).
pub fn wire(root: &Path, model: &ProjectModel, package: &str, steps: &[String]) -> Result<Changeset> {
    let Some(r) = recipe(package) else { bail!("NestFactory has no wiring for {package}") };
    let (name, file) = root_module(model);
    let mut tree = Tree::with_esm(root, model.esm);
    let mut src = tree.read(&file).unwrap_or_default();
    let mut notes = vec![];
    if steps.iter().any(|s| s == "module") && !is_wired(root, model, r) {
        for (symbol, from) in r.imports {
            src = edit::insert_import(&src, symbol, from);
        }
        for (metadata, element, _) in r.entries {
            src = edit::insert_metadata(&src, metadata, element).unwrap_or(src);
        }
        notes.push(format!("{} is wired into {name} once; nothing else in the file changes.", r.package));
    }
    if let (true, Some(ex)) = (steps.iter().any(|s| s == "example"), &r.example) {
        let path = format!("{}/{}", model.source_root, ex.path);
        if !tree.exists(&path) {
            tree.write(&path, ex.content);
            let rel = relative_import(&file, &path);
            let rel = if model.esm { format!("{rel}.js") } else { rel };
            src = edit::declare(&src, ex.metadata, ex.class, &rel);
        }
    }
    tree.write(&file, src);
    let mut cs = tree.into_changeset(format!("Wire {package}"), None);
    cs.notes = notes;
    Ok(cs)
}

/// The changeset that takes the recipe's wiring out of the root module, or `None` when
/// there's nothing to take out. Example files stay: they're the user's code now.
pub fn unwire(root: &Path, model: &ProjectModel, package: &str) -> Option<Changeset> {
    let r = recipe(package)?;
    let (name, file) = root_module(model);
    let mut tree = Tree::with_esm(root, model.esm);
    let before = tree.read(&file)?;
    let mut src = before.clone();
    for (metadata, _, key) in r.entries {
        if let Some(next) = edit::remove_metadata(&src, metadata, key) {
            src = next;
        }
    }
    for (symbol, _) in r.imports {
        src = edit::remove_unused_import(&src, symbol);
    }
    if src == before {
        return None;
    }
    tree.write(&file, src);
    let mut cs = tree.into_changeset(format!("Unwire {package}"), None);
    cs.notes = vec![format!("Takes {package}'s wiring out of {name} before uninstalling.")];
    Some(cs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset;
    use crate::Scanner;

    const APP: &str = "import { Module } from '@nestjs/common';\nimport { AppService } from './app.service';\n\n@Module({\n  imports: [],\n  providers: [AppService],\n})\nexport class AppModule {}\n";

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{ "name": "x", "dependencies": { "@nestjs/core": "^12.0.0" } }"#).unwrap();
        std::fs::write(dir.path().join("src/app.module.ts"), APP).unwrap();
        std::fs::write(dir.path().join("src/app.service.ts"), "import { Injectable } from '@nestjs/common';\n\n@Injectable()\nexport class AppService {}\n").unwrap();
        std::fs::write(dir.path().join("src/main.ts"), "import { NestFactory } from '@nestjs/core';\nimport { AppModule } from './app.module';\n\nasync function bootstrap() {\n  const app = await NestFactory.create(AppModule);\n  await app.listen(3000);\n}\nbootstrap();\n").unwrap();
        dir
    }

    #[test]
    fn wires_and_unwires_schedule() {
        let dir = project();
        let model = Scanner::new().scan(dir.path());
        let steps = steps(dir.path(), &model, "@nestjs/schedule");
        assert_eq!(steps.iter().map(|s| (s.id.as_str(), s.done)).collect::<Vec<_>>(), [("module", false), ("example", false)]);
        assert!(steps[0].label.contains("`AppModule`"));

        let cs = wire(dir.path(), &model, "@nestjs/schedule", &["module".into(), "example".into()]).unwrap();
        let app = cs.files.iter().find(|f| f.path == "src/app.module.ts").unwrap().after.clone().unwrap();
        assert!(app.contains("import { ScheduleModule } from '@nestjs/schedule';"), "{app}");
        assert!(app.contains("import { TasksService } from './tasks/tasks.service';"), "{app}");
        assert!(app.contains("imports: [ScheduleModule.forRoot()],") && app.contains("providers: [AppService, TasksService],"), "{app}");
        assert!(cs.files.iter().any(|f| f.path == "src/tasks/tasks.service.ts"));
        changeset::apply(dir.path(), &cs).unwrap();

        let model = Scanner::new().scan(dir.path());
        assert!(super::steps(dir.path(), &model, "@nestjs/schedule").iter().all(|s| s.done));
        let undo = unwire(dir.path(), &model, "@nestjs/schedule").unwrap();
        let app = undo.files[0].after.clone().unwrap();
        assert!(!app.contains("ScheduleModule") && app.contains("imports: [],") && app.contains("TasksService"), "{app}");
    }

    #[test]
    fn global_validation_pipe_counts_as_wired_from_main() {
        let dir = project();
        let main = dir.path().join("src/main.ts");
        let src = std::fs::read_to_string(&main).unwrap().replace("await app.listen", "app.useGlobalPipes(new ValidationPipe());\n  await app.listen");
        std::fs::write(&main, src).unwrap();
        let model = Scanner::new().scan(dir.path());
        assert!(steps(dir.path(), &model, "class-validator")[0].done);
        assert!(steps(dir.path(), &model, "helmet").is_empty());
        assert!(unwire(dir.path(), &model, "@nestjs/schedule").is_none());
    }
}
