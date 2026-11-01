//! Package manager commands.

use crate::model::PackageManager;

pub fn program(pm: PackageManager) -> &'static str {
    match pm {
        PackageManager::Npm => "npm",
        PackageManager::Pnpm => "pnpm",
        PackageManager::Yarn => "yarn",
        PackageManager::Bun => "bun",
    }
}

pub fn add(pm: PackageManager, packages: &[String], dev: bool) -> Vec<String> {
    let mut cmd = vec![program(pm).to_string()];
    cmd.push(if pm == PackageManager::Npm { "install" } else { "add" }.to_string());
    if dev {
        cmd.push("-D".into());
    }
    cmd.extend(packages.iter().cloned());
    cmd
}

pub fn install(pm: PackageManager) -> Vec<String> {
    vec![program(pm).to_string(), "install".to_string()]
}

pub fn run_script(pm: PackageManager, script: &str) -> Vec<String> {
    match pm {
        PackageManager::Yarn => vec!["yarn".into(), script.into()],
        _ => vec![program(pm).into(), "run".into(), script.into()],
    }
}

/// `npx`-style execution of a local binary.
pub fn exec(pm: PackageManager, bin: &str, args: &[&str]) -> Vec<String> {
    let mut cmd: Vec<String> = match pm {
        PackageManager::Npm => vec!["npx".into(), "--no-install".into()],
        PackageManager::Pnpm => vec!["pnpm".into(), "exec".into()],
        PackageManager::Yarn => vec!["yarn".into()],
        PackageManager::Bun => vec!["bunx".into()],
    };
    cmd.push(bin.into());
    cmd.extend(args.iter().map(|s| s.to_string()));
    cmd
}

pub fn display(cmd: &[String]) -> String {
    cmd.iter()
        .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}
