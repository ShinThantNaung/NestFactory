use std::path::{Path, PathBuf};

use nestfactory_core::changeset::{apply, undo, ChangeStatus};
use nestfactory_core::fixes::preview_fix;
use nestfactory_core::generate::{generate, GenerateOptions, Schematic, Transport};
use nestfactory_core::project::read_meta;
use nestfactory_core::*;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/orders-api")
}

fn copy_dir(from: &Path, to: &Path) {
    for e in ignore::WalkBuilder::new(from).hidden(false).build().flatten() {
        let rel = e.path().strip_prefix(from).unwrap();
        let dest = to.join(rel);
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
        } else {
            std::fs::copy(e.path(), &dest).unwrap();
        }
    }
}

fn opts(schematic: Schematic, name: &str) -> GenerateOptions {
    GenerateOptions {
        schematic,
        name: name.into(),
        directory: None,
        transport: Transport::Rest,
        crud: true,
        spec: true,
        flat: false,
        swagger: false,
        skip_import: false,
        fields: vec![],
    }
}

#[test]
fn resource_matches_cli_layout_and_wiring() {
    let root = fixture();
    let cs = generate(&root, &read_meta(&root), &opts(Schematic::Resource, "invoices")).unwrap();
    let paths: Vec<_> = cs.files.iter().map(|f| (f.status, f.path.as_str())).collect();
    assert_eq!(
        paths,
        vec![
            (ChangeStatus::Modified, "src/app.module.ts"),
            (ChangeStatus::Added, "src/invoices/dto/create-invoice.dto.ts"),
            (ChangeStatus::Added, "src/invoices/dto/update-invoice.dto.ts"),
            (ChangeStatus::Added, "src/invoices/entities/invoice.entity.ts"),
            (ChangeStatus::Added, "src/invoices/invoices.controller.ts"),
            (ChangeStatus::Added, "src/invoices/invoices.module.ts"),
            (ChangeStatus::Added, "src/invoices/invoices.service.ts"),
            (ChangeStatus::Added, "src/invoices/invoices.controller.spec.ts"),
            (ChangeStatus::Added, "src/invoices/invoices.service.spec.ts"),
        ]
    );
    assert_eq!(cs.command.as_deref(), Some("nest g resource invoices --type rest --crud"));
    let app = cs.files[0].after.as_deref().unwrap();
    assert!(app.contains("import { InvoicesModule } from './invoices/invoices.module';\n\n@Module"));
    assert!(app.contains("    OrdersModule,\n    InvoicesModule,\n  ],"));
    assert_eq!(cs.files[0].additions, 2);
    // The fixture has @nestjs/swagger, so PartialType comes from it, like the CLI.
    let update = cs.files.iter().find(|f| f.path.ends_with("update-invoice.dto.ts")).unwrap();
    assert!(update.after.as_deref().unwrap().starts_with("import { PartialType } from '@nestjs/swagger';"));
    let ctrl = cs.files.iter().find(|f| f.path.ends_with("invoices.controller.ts")).unwrap().after.clone().unwrap();
    assert!(ctrl.contains("@Controller('invoices')"));
    assert!(ctrl.contains("create(@Body() createInvoiceDto: CreateInvoiceDto)"));
}

#[test]
fn service_in_module_folder_is_wired_into_that_module() {
    let root = fixture();
    let mut o = opts(Schematic::Service, "pricing");
    o.directory = Some("src/orders".into());
    o.spec = false;
    let cs = generate(&root, &read_meta(&root), &o).unwrap();
    assert_eq!(cs.files[0].path, "src/orders/orders.module.ts");
    assert!(cs.files[0].after.as_deref().unwrap().contains("providers: [OrdersService, PricingService],"));
    assert!(cs.files[0].after.as_deref().unwrap().contains("import { PricingService } from './pricing/pricing.service';"));
    assert_eq!(cs.files[1].path, "src/orders/pricing/pricing.service.ts");
    assert_eq!(cs.command.as_deref(), Some("nest g service orders/pricing --no-spec"));
}

#[test]
fn refuses_to_overwrite() {
    let root = fixture();
    let err = generate(&root, &read_meta(&root), &opts(Schematic::Resource, "orders")).unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");
}

#[test]
fn guard_and_flat_options() {
    let root = fixture();
    let mut o = opts(Schematic::Guard, "roles");
    o.flat = true;
    let cs = generate(&root, &read_meta(&root), &o).unwrap();
    let paths: Vec<_> = cs.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["src/roles.guard.ts", "src/roles.guard.spec.ts"]);
    assert!(cs.files[0].after.as_deref().unwrap().contains("export class RolesGuard implements CanActivate"));
}

#[test]
fn every_schematic_renders() {
    let root = fixture();
    let meta = read_meta(&root);
    for s in [
        Schematic::Module, Schematic::Controller, Schematic::Service, Schematic::Provider, Schematic::Guard,
        Schematic::Pipe, Schematic::Interceptor, Schematic::Filter, Schematic::Middleware, Schematic::Gateway,
        Schematic::Resolver, Schematic::Decorator, Schematic::Class, Schematic::Interface, Schematic::Dto,
    ] {
        let cs = generate(&root, &meta, &opts(s, "widgets")).unwrap_or_else(|e| panic!("{s:?}: {e}"));
        assert!(!cs.files.is_empty(), "{s:?}");
        for f in &cs.files {
            let text = f.after.as_deref().unwrap();
            assert!(!text.contains("<%"), "{s:?} {}: unrendered template", f.path);
        }
    }
    for t in [Transport::GraphqlCodeFirst, Transport::GraphqlSchemaFirst, Transport::Microservice, Transport::Ws] {
        let mut o = opts(Schematic::Resource, "widgets");
        o.transport = t;
        generate(&root, &meta, &o).unwrap_or_else(|e| panic!("{t:?}: {e}"));
    }
}

#[test]
fn fixes_apply_and_undo_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture(), dir.path());
    let model = Scanner::new().scan(dir.path());
    let missing = model.diagnostics.iter().find(|d| d.code == DiagnosticCode::MissingProvider).unwrap();
    let cs = preview_fix(dir.path(), &model, &missing.fixes[0].action).unwrap();
    assert_eq!(cs.files.len(), 1);
    apply(dir.path(), &cs).unwrap();
    let after = Scanner::new().scan(dir.path());
    assert!(!after.diagnostics.iter().any(|d| d.code == DiagnosticCode::MissingProvider));

    let cycle = after.diagnostics.iter().find(|d| d.code == DiagnosticCode::CircularImport).unwrap();
    let cs2 = preview_fix(dir.path(), &after, &cycle.fixes[0].action).unwrap();
    assert_eq!(cs2.files.len(), 2);
    assert!(cs2.files.iter().any(|f| f.after.as_deref().unwrap().contains("forwardRef(() => PaymentsModule)")));
    apply(dir.path(), &cs2).unwrap();
    let fixed = Scanner::new().scan(dir.path());
    assert!(fixed.diagnostics.iter().all(|d| d.severity != Severity::Error && d.code != DiagnosticCode::CircularImport), "{:?}", fixed.diagnostics);

    undo(dir.path(), &cs2).unwrap();
    undo(dir.path(), &cs).unwrap();
    let back = Scanner::new().scan(dir.path());
    assert_eq!(back.diagnostics.len(), 2);
}
