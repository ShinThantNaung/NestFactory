//! New project wizard: wraps `nest new`, then installs and writes the starter modules
//! (config, auth, Swagger, health). Everything it writes builds without NestFactory.

use std::path::Path;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::changeset::{Changeset, Tree};
use crate::edit;
use crate::env;
use crate::model::PackageManager;
use crate::pm;
use crate::project::{import_spec, read_meta};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WizardOptions {
    pub name: String,
    /// Parent folder the project is created in.
    pub location: String,
    pub monorepo: bool,
    pub package_manager: PackageManager,
    pub strict: bool,
    pub lint: bool,
    pub config: bool,
    pub auth: bool,
    pub swagger: bool,
    pub health: bool,
    /// Open the database connection manager after creating.
    pub database: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WizardStep {
    pub title: String,
    /// `None` for the starter-module recipe NestFactory applies itself.
    pub command: Option<Vec<String>>,
    /// Run in the project folder (otherwise in the parent location).
    pub in_project: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlannedFile {
    pub path: String,
    pub added: bool,
    pub note: Option<String>,
}

pub fn validate(opts: &WizardOptions) -> Result<()> {
    let n = &opts.name;
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') || n.starts_with('-') {
        bail!("Use lowercase letters, digits, '-' or '_' for the project name");
    }
    if opts.location.trim().is_empty() {
        bail!("Choose a location");
    }
    if Path::new(&opts.location).join(n).exists() {
        bail!("{} already exists in that folder", n);
    }
    Ok(())
}

fn starter_packages(opts: &WizardOptions) -> (Vec<String>, Vec<String>) {
    let mut deps: Vec<&str> = vec![];
    let mut dev: Vec<&str> = vec![];
    if opts.config || opts.auth {
        deps.extend(["@nestjs/config", "class-validator", "class-transformer"]);
    }
    if opts.swagger {
        deps.push("@nestjs/swagger");
    }
    if opts.health {
        deps.push("@nestjs/terminus");
    }
    if opts.auth {
        deps.extend(["@nestjs/jwt", "@nestjs/passport", "passport", "passport-jwt"]);
        dev.push("@types/passport-jwt");
    }
    (deps.into_iter().map(String::from).collect(), dev.into_iter().map(String::from).collect())
}

pub fn plan(opts: &WizardOptions) -> Vec<WizardStep> {
    let mut steps = vec![];
    let mut new_cmd: Vec<String> = vec!["nest".into(), "new".into(), opts.name.clone(), "--package-manager".into(), pm::program(opts.package_manager).into()];
    if opts.strict {
        new_cmd.push("--strict".into());
    }
    steps.push(WizardStep { title: "Create the project with the Nest CLI".into(), command: Some(new_cmd), in_project: false });
    if opts.monorepo {
        steps.push(WizardStep {
            title: "Switch to a monorepo with a shared library".into(),
            command: Some(vec!["nest".into(), "generate".into(), "library".into(), "common".into(), "--prefix".into(), "@app".into()]),
            in_project: true,
        });
    }
    let (deps, dev) = starter_packages(opts);
    if !deps.is_empty() {
        steps.push(WizardStep { title: "Install starter packages".into(), command: Some(pm::add(opts.package_manager, &deps, false)), in_project: true });
    }
    if !dev.is_empty() {
        steps.push(WizardStep { title: "Install type definitions".into(), command: Some(pm::add(opts.package_manager, &dev, true)), in_project: true });
    }
    steps.push(WizardStep { title: "Write starter modules".into(), command: None, in_project: true });
    steps
}

pub fn planned_files(opts: &WizardOptions) -> Vec<PlannedFile> {
    let src = if opts.monorepo { format!("apps/{}/src", opts.name) } else { "src".into() };
    let f = |p: String, added: bool, note: Option<&str>| PlannedFile { path: p, added, note: note.map(String::from) };
    let mut out = vec![
        f(format!("{src}/main.ts"), false, if opts.swagger { Some("Swagger at /docs") } else { None }),
        f(format!("{src}/app.module.ts"), false, None),
    ];
    if opts.config || opts.auth {
        out.push(f(format!("{src}/config/env.validation.ts"), true, None));
    }
    if opts.health {
        out.push(f(format!("{src}/health/health.controller.ts"), true, None));
    }
    if opts.auth {
        out.push(f(format!("{src}/auth/auth.module.ts"), true, Some("JWT + Passport")));
    }
    if opts.monorepo {
        out.push(f("libs/common/src/common.module.ts".into(), true, None));
    }
    out.push(f("test/app.e2e-spec.ts".into(), false, None));
    if opts.config || opts.auth {
        out.push(f(".env".into(), true, Some("gitignored")));
    }
    out.push(f("nest-cli.json · package.json · tsconfig.json".into(), false, None));
    out
}

const ENV_VALIDATION: &str = "import { plainToInstance } from 'class-transformer';
import { IsEnum, IsNumber, IsOptional, validateSync } from 'class-validator';

enum Environment {
  Development = 'development',
  Production = 'production',
  Test = 'test',
}

class EnvironmentVariables {
  @IsEnum(Environment)
  @IsOptional()
  NODE_ENV: Environment = Environment.Development;

  @IsNumber()
  @IsOptional()
  PORT: number = 3000;
}

export function validate(config: Record<string, unknown>) {
  const validated = plainToInstance(EnvironmentVariables, config, {
    enableImplicitConversion: true,
  });
  const errors = validateSync(validated, { skipMissingProperties: false });
  if (errors.length > 0) {
    throw new Error(errors.toString());
  }
  return validated;
}
";

const HEALTH_MODULE: &str = "import { Module } from '@nestjs/common';
import { TerminusModule } from '@nestjs/terminus';
import { HealthController } from './health.controller';

@Module({
  imports: [TerminusModule],
  controllers: [HealthController],
})
export class HealthModule {}
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
    return this.health.check([
      () => this.memory.checkHeap('memory_heap', 512 * 1024 * 1024),
    ]);
  }
}
";

const AUTH_MODULE: &str = "import { Module } from '@nestjs/common';
import { ConfigService } from '@nestjs/config';
import { JwtModule } from '@nestjs/jwt';
import { PassportModule } from '@nestjs/passport';
import { AuthController } from './auth.controller';
import { AuthService } from './auth.service';
import { JwtStrategy } from './jwt.strategy';

@Module({
  imports: [
    PassportModule,
    JwtModule.registerAsync({
      inject: [ConfigService],
      useFactory: (config: ConfigService) => ({
        secret: config.getOrThrow<string>('JWT_SECRET'),
        signOptions: { expiresIn: '1h' },
      }),
    }),
  ],
  controllers: [AuthController],
  providers: [AuthService, JwtStrategy],
  exports: [AuthService],
})
export class AuthModule {}
";

const AUTH_SERVICE: &str = "import { Injectable, UnauthorizedException } from '@nestjs/common';
import { JwtService } from '@nestjs/jwt';

interface AuthUser {
  id: string;
  username: string;
}

@Injectable()
export class AuthService {
  constructor(private readonly jwt: JwtService) {}

  async signIn(username: string, password: string) {
    const user = await this.validateUser(username, password);
    if (!user) {
      throw new UnauthorizedException();
    }
    return {
      access_token: await this.jwt.signAsync({ sub: user.id, username: user.username }),
    };
  }

  // Look the user up and compare a password hash here (for example with bcrypt).
  // Until then every sign-in is rejected.
  private validateUser(username: string, password: string): Promise<AuthUser | null> {
    void username;
    void password;
    return Promise.resolve(null);
  }
}
";

const AUTH_CONTROLLER: &str = "import { Body, Controller, Get, HttpCode, HttpStatus, Post, Request, UseGuards } from '@nestjs/common';
import { AuthService } from './auth.service';
import { SignInDto } from './dto/sign-in.dto';
import { JwtAuthGuard } from './jwt-auth.guard';

@Controller('auth')
export class AuthController {
  constructor(private readonly authService: AuthService) {}

  @HttpCode(HttpStatus.OK)
  @Post('login')
  signIn(@Body() dto: SignInDto) {
    return this.authService.signIn(dto.username, dto.password);
  }

  @UseGuards(JwtAuthGuard)
  @Get('profile')
  profile(@Request() req: { user: unknown }) {
    return req.user;
  }
}
";

const JWT_STRATEGY: &str = "import { Injectable } from '@nestjs/common';
import { ConfigService } from '@nestjs/config';
import { PassportStrategy } from '@nestjs/passport';
import { ExtractJwt, Strategy } from 'passport-jwt';

@Injectable()
export class JwtStrategy extends PassportStrategy(Strategy) {
  constructor(config: ConfigService) {
    super({
      jwtFromRequest: ExtractJwt.fromAuthHeaderAsBearerToken(),
      ignoreExpiration: false,
      secretOrKey: config.getOrThrow<string>('JWT_SECRET'),
    });
  }

  validate(payload: { sub: string; username: string }) {
    return { userId: payload.sub, username: payload.username };
  }
}
";

const JWT_GUARD: &str = "import { Injectable } from '@nestjs/common';
import { AuthGuard } from '@nestjs/passport';

@Injectable()
export class JwtAuthGuard extends AuthGuard('jwt') {}
";

const SIGN_IN_DTO: &str = "import { IsString } from 'class-validator';

export class SignInDto {
  @IsString()
  username: string;

  @IsString()
  password: string;
}
";

/// A local development secret from OS-seeded hash keys.
pub fn dev_secret() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..4u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// Inserts lines right after the `NestFactory.create(...)` statement in main.ts.
fn insert_after_create(main: &str, lines: &str) -> String {
    let Some(at) = main.find("NestFactory.create") else { return main.to_string() };
    let Some(semi) = main[at..].find(';') else { return main.to_string() };
    let end = at + semi + 1;
    let nl = main[end..].find('\n').map(|i| end + i + 1).unwrap_or(main.len());
    format!("{}{}{}", &main[..nl], lines, &main[nl..])
}

/// The starter-module recipe, applied after `nest new` and the installs.
pub fn starter_changeset(root: &Path, opts: &WizardOptions) -> Result<Changeset> {
    let meta = read_meta(root);
    let sr = meta.source_root.clone();
    let app_module = format!("{sr}/app.module.ts");
    let main_file = format!("{sr}/main.ts");
    let mut tree = Tree::with_esm(root, meta.esm);
    let mut app = tree.read(&app_module).unwrap_or_default();
    let mut main = tree.read(&main_file).unwrap_or_default();
    let config = opts.config || opts.auth;

    if config {
        tree.write(&format!("{sr}/config/env.validation.ts"), ENV_VALIDATION);
        app = edit::insert_import(&app, "ConfigModule", "@nestjs/config");
        app = edit::insert_import(&app, "validate", if meta.esm { "./config/env.validation.js" } else { "./config/env.validation" });
        app = edit::insert_metadata(&app, "imports", "ConfigModule.forRoot({ isGlobal: true, validate })").unwrap_or(app);
        main = edit::insert_import(&main, "ValidationPipe", "@nestjs/common");
        main = insert_after_create(&main, "  app.useGlobalPipes(new ValidationPipe({ whitelist: true }));\n");
        let mut env_updates = vec![("NODE_ENV".to_string(), Some("development".to_string())), ("PORT".to_string(), Some("3000".to_string()))];
        if opts.auth {
            env_updates.push(("JWT_SECRET".into(), Some(dev_secret())));
        }
        tree.write(".env", env::update_env("# Local development settings. Not committed.\n", &env_updates));
        let gi = tree.read(".gitignore").unwrap_or_default();
        if !gi.lines().any(|l| l.trim() == ".env") {
            tree.write(".gitignore", format!("{gi}\n# local environment\n.env\n"));
        }
    }
    if opts.auth {
        let v = format!("{sr}/config/env.validation.ts");
        let src = tree.read(&v).unwrap_or_default();
        tree.write(&v, env::add_schema_property(&src, "JWT_SECRET", "string"));
        for (name, body) in [
            ("auth.module.ts", AUTH_MODULE),
            ("auth.service.ts", AUTH_SERVICE),
            ("auth.controller.ts", AUTH_CONTROLLER),
            ("jwt.strategy.ts", JWT_STRATEGY),
            ("jwt-auth.guard.ts", JWT_GUARD),
            ("dto/sign-in.dto.ts", SIGN_IN_DTO),
        ] {
            tree.write(&format!("{sr}/auth/{name}"), body);
        }
        app = edit::declare(&app, "imports", "AuthModule", &import_spec(&app_module, &format!("{sr}/auth/auth.module.ts"), meta.esm));
    }
    if opts.health {
        tree.write(&format!("{sr}/health/health.module.ts"), HEALTH_MODULE);
        tree.write(&format!("{sr}/health/health.controller.ts"), HEALTH_CONTROLLER);
        app = edit::declare(&app, "imports", "HealthModule", &import_spec(&app_module, &format!("{sr}/health/health.module.ts"), meta.esm));
    }
    if opts.swagger {
        main = edit::insert_import(&main, "DocumentBuilder", "@nestjs/swagger");
        main = edit::insert_import(&main, "SwaggerModule", "@nestjs/swagger");
        let block = format!(
            "\n  const config = new DocumentBuilder()\n    .setTitle('{}')\n    .setVersion('1.0')\n    .build();\n  const document = SwaggerModule.createDocument(app, config);\n  SwaggerModule.setup('docs', app, document);\n",
            opts.name
        );
        main = match main.find("  await app.listen") {
            Some(i) => format!("{}{}\n{}", main[..i].trim_end_matches('\n'), block, &main[i..]),
            None => insert_after_create(&main, &block),
        };
    }
    tree.write(&app_module, app);
    tree.write(&main_file, main);

    if !opts.lint {
        for f in ["eslint.config.mjs", ".eslintrc.js", ".prettierrc", "oxlint.json", ".oxlintrc.json"] {
            if tree.exists(f) {
                tree.delete(f);
            }
        }
        if let Some(pkg) = tree.read("package.json") {
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&pkg) {
                if let Some(dev) = v.get_mut("devDependencies").and_then(|d| d.as_object_mut()) {
                    dev.retain(|k, _| !(k.contains("eslint") || k.contains("prettier") || k.contains("oxlint") || k == "globals"));
                }
                if let Some(scripts) = v.get_mut("scripts").and_then(|d| d.as_object_mut()) {
                    scripts.remove("lint");
                    scripts.remove("format");
                }
                if let Ok(s) = serde_json::to_string_pretty(&v) {
                    tree.write("package.json", format!("{s}\n"));
                }
            }
        }
    }
    let mut cs = tree.into_changeset(format!("Starter modules for {}", opts.name), None);
    cs.notes.push("No NestFactory files are added. The project matches Nest CLI output.".into());
    Ok(cs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "import { NestFactory } from '@nestjs/core';\nimport { AppModule } from './app.module';\n\nasync function bootstrap() {\n  const app = await NestFactory.create(AppModule);\n  await app.listen(process.env.PORT ?? 3000);\n}\nbootstrap();\n";
    const APP: &str = "import { Module } from '@nestjs/common';\nimport { AppController } from './app.controller';\nimport { AppService } from './app.service';\n\n@Module({\n  imports: [],\n  controllers: [AppController],\n  providers: [AppService],\n})\nexport class AppModule {}\n";

    #[test]
    fn starters_wire_into_a_fresh_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.ts"), MAIN).unwrap();
        std::fs::write(dir.path().join("src/app.module.ts"), APP).unwrap();
        std::fs::write(dir.path().join("package.json"), "{\"name\":\"orders-api\",\"devDependencies\":{\"eslint\":\"^9\",\"jest\":\"^29\"},\"scripts\":{\"lint\":\"eslint\",\"test\":\"jest\"}}").unwrap();
        let opts = WizardOptions {
            name: "orders-api".into(),
            location: String::new(),
            monorepo: false,
            package_manager: PackageManager::Npm,
            strict: true,
            lint: false,
            config: true,
            auth: true,
            swagger: true,
            health: true,
            database: false,
        };
        let cs = starter_changeset(dir.path(), &opts).unwrap();
        let get = |p: &str| cs.files.iter().find(|f| f.path == p).and_then(|f| f.after.clone()).unwrap();
        let app = get("src/app.module.ts");
        assert!(app.contains("imports: [ConfigModule.forRoot({ isGlobal: true, validate }), AuthModule, HealthModule],"), "{app}");
        let main = get("src/main.ts");
        assert!(main.contains("app.useGlobalPipes(new ValidationPipe({ whitelist: true }));"));
        assert!(main.contains("import { DocumentBuilder, SwaggerModule } from '@nestjs/swagger';"), "{main}");
        assert!(main.contains("  SwaggerModule.setup('docs', app, document);\n\n  await app.listen"), "{main}");
        assert!(get(".env").contains("JWT_SECRET="));
        assert!(get("src/config/env.validation.ts").contains("JWT_SECRET: string;"));
        assert!(!get("package.json").contains("eslint"));
    }
}
