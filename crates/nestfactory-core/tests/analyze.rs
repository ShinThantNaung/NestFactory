use std::path::PathBuf;

use nestfactory_core::*;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn scan() -> ProjectModel {
    Scanner::new().scan(&fixture("orders-api"))
}

#[test]
fn reads_project_metadata() {
    let m = scan();
    assert_eq!(m.name, "orders-api");
    assert_eq!(m.nest_version.as_deref(), Some("11"));
    assert_eq!(m.root_modules, vec!["AppModule"]);
    assert_eq!(m.port, 3000);
    assert_eq!(m.modules.len(), 7);
}

#[test]
fn reads_module_metadata() {
    let m = scan();
    let orders = m.modules.iter().find(|x| x.name == "OrdersModule").unwrap();
    let imports: Vec<_> = orders.imports.iter().map(|i| i.display.as_str()).collect();
    assert_eq!(imports, vec!["TypeOrmModule.forFeature", "UsersModule", "PaymentsModule"]);
    assert_eq!(orders.imports[0].features, vec!["Order"]);
    assert_eq!(orders.controllers, vec!["OrdersController"]);
    assert_eq!(orders.exports, vec!["OrdersService"]);
    assert_eq!(orders.orm.as_deref(), Some("TypeORM"));
}

#[test]
fn resolves_injections() {
    let m = scan();
    let svc = m.classes.iter().find(|c| c.name == "OrdersService").unwrap();
    assert_eq!(svc.kind, ClassKind::Service);
    assert_eq!(svc.injects.len(), 2);
    assert_eq!(svc.injects[0].token, "Repository<Order>");
    assert_eq!(svc.injects[0].status, InjectionStatus::Resolved);
    assert_eq!(svc.injects[1].via.as_deref(), Some("UsersModule"));
    let auth = m.classes.iter().find(|c| c.name == "AuthService").unwrap();
    // ConfigModule.forRoot({ isGlobal: true }) makes ConfigService available everywhere.
    assert_eq!(auth.injects[1].status, InjectionStatus::Resolved);
}

#[test]
fn finds_routes() {
    let m = scan();
    let routes: Vec<_> = m.routes.iter().map(|r| format!("{} {}", r.method, r.path)).collect();
    assert_eq!(routes, vec!["POST /orders", "GET /orders", "GET /orders/:id", "PATCH /orders/:id", "DELETE /orders/:id"]);
}

#[test]
fn reports_the_design_issues() {
    let m = scan();
    let codes: Vec<_> = m.diagnostics.iter().map(|d| (d.code, d.subject.clone())).collect();
    assert_eq!(
        codes,
        vec![
            (DiagnosticCode::MissingProvider, "PaymentsService → MailService".to_string()),
            (DiagnosticCode::CircularImport, "OrdersModule ⇄ PaymentsModule".to_string()),
        ]
    );
    let missing = &m.diagnostics[0];
    assert_eq!(
        missing.fixes[0].action,
        FixAction::ImportModule { module: "MailModule".into(), into: "PaymentsModule".into(), package: None }
    );
    let mail = m.graph.nodes.iter().find(|n| n.id == "MailModule").unwrap();
    assert_eq!(mail.status, NodeStatus::Orphan);
    assert!(m.graph.edges.iter().any(|e| e.from == "PaymentsModule" && e.to == "MailModule" && e.kind == EdgeKind::Missing));
    assert!(m.graph.edges.iter().any(|e| e.from == "OrdersModule" && e.to == "PaymentsModule" && e.kind == EdgeKind::Cycle));
}

#[test]
fn lays_out_graph_top_down() {
    let m = scan();
    let y = |id: &str| m.graph.nodes.iter().find(|n| n.id == id).unwrap().y;
    assert!(y("AppModule") < y("OrdersModule"));
    assert!(y("OrdersModule") < y("UsersModule"));
}
