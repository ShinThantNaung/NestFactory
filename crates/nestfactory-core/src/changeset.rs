//! Changesets: every generator, fix and setup step produces one. It is shown as a diff
//! first, written as a single step, and undone as a single step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileChange {
    pub path: String,
    pub status: ChangeStatus,
    pub before: Option<String>,
    pub after: Option<String>,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Changeset {
    pub id: String,
    pub label: String,
    /// The equivalent Nest CLI command, when there is one.
    pub command: Option<String>,
    pub files: Vec<FileChange>,
    pub notes: Vec<String>,
    /// Packages to install after writing, as `name` or `name@range`.
    pub install: Vec<String>,
    pub install_dev: Vec<String>,
    /// Run a plain package install after writing (package.json was edited).
    pub run_install: bool,
}

impl Changeset {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.install.is_empty() && self.install_dev.is_empty()
    }
}

/// An in-memory overlay over the project directory, like the schematics `Tree`.
pub struct Tree {
    root: PathBuf,
    overlay: BTreeMap<String, Option<String>>,
    esm: bool,
}

/// Adds `.js` to extensionless relative imports, as ESM (`"type": "module"`) projects need.
pub fn esmify(content: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r#"(from\s+|import\s+)'(\.{1,2}/[^']+?)'"#).unwrap());
    re.replace_all(content, |c: &regex::Captures| {
        let spec = &c[2];
        if spec.ends_with(".js") || spec.ends_with(".json") || spec.ends_with(".mjs") || spec.ends_with(".cjs") {
            c[0].to_string()
        } else {
            format!("{}'{spec}.js'", &c[1])
        }
    })
    .into_owned()
}

impl Tree {
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf(), overlay: BTreeMap::new(), esm: false }
    }

    /// New `.ts` files written to an ESM project get `.js` import suffixes.
    pub fn with_esm(root: &Path, esm: bool) -> Self {
        Self { root: root.to_path_buf(), overlay: BTreeMap::new(), esm }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn disk(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(rel)).ok()
    }

    pub fn read(&self, rel: &str) -> Option<String> {
        match self.overlay.get(rel) {
            Some(v) => v.clone(),
            None => self.disk(rel),
        }
    }

    pub fn exists(&self, rel: &str) -> bool {
        match self.overlay.get(rel) {
            Some(v) => v.is_some(),
            None => self.root.join(rel).is_file(),
        }
    }

    pub fn write(&mut self, rel: &str, content: impl Into<String>) {
        let mut content = content.into();
        if self.esm && rel.ends_with(".ts") && !self.root.join(rel).is_file() {
            content = esmify(&content);
        }
        self.overlay.insert(rel.to_string(), Some(content));
    }

    pub fn delete(&mut self, rel: &str) {
        self.overlay.insert(rel.to_string(), None);
    }

    /// File names directly inside `dir` (project-relative), including pending writes.
    pub fn list_files(&self, dir: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        if let Ok(rd) = std::fs::read_dir(self.root.join(dir)) {
            for e in rd.flatten() {
                if e.file_type().is_ok_and(|t| t.is_file()) {
                    out.push(e.file_name().to_string_lossy().to_string());
                }
            }
        }
        let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
        for (path, v) in &self.overlay {
            if let Some(name) = path.strip_prefix(&prefix) {
                if !name.contains('/') {
                    match v {
                        Some(_) if !out.iter().any(|n| n == name) => out.push(name.to_string()),
                        None => out.retain(|n| n != name),
                        _ => {}
                    }
                }
            }
        }
        out.sort();
        out
    }

    pub fn into_changeset(self, label: impl Into<String>, command: Option<String>) -> Changeset {
        let mut files = vec![];
        for (path, after) in &self.overlay {
            let before = self.disk(path);
            if before == *after {
                continue;
            }
            let status = match (&before, after) {
                (None, Some(_)) => ChangeStatus::Added,
                (Some(_), None) => ChangeStatus::Deleted,
                _ => ChangeStatus::Modified,
            };
            let (additions, deletions) = line_counts(before.as_deref().unwrap_or(""), after.as_deref().unwrap_or(""));
            files.push(FileChange { path: path.clone(), status, before, after: after.clone(), additions, deletions });
        }
        // Modified files first (the wiring), then new files in path order, like the design.
        files.sort_by(|a, b| {
            let rank = |s: ChangeStatus| match s {
                ChangeStatus::Modified => 0,
                ChangeStatus::Added => 1,
                ChangeStatus::Deleted => 2,
            };
            rank(a.status).cmp(&rank(b.status)).then_with(|| spec_last(&a.path).cmp(&spec_last(&b.path))).then_with(|| a.path.cmp(&b.path))
        });
        Changeset { id: new_id(), label: label.into(), command, files, ..Default::default() }
    }
}

fn spec_last(path: &str) -> bool {
    path.contains(".spec.") || path.contains(".e2e-spec.")
}

pub fn line_counts(before: &str, after: &str) -> (u32, u32) {
    let diff = TextDiff::from_lines(before, after);
    let mut add = 0;
    let mut del = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => add += 1,
            ChangeTag::Delete => del += 1,
            ChangeTag::Equal => {}
        }
    }
    (add, del)
}

pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("cs-{t:x}-{}", N.fetch_add(1, Ordering::Relaxed))
}

fn read_opt(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Writes a changeset. Refuses if any file changed since the preview was computed.
pub fn apply(root: &Path, cs: &Changeset) -> Result<()> {
    for f in &cs.files {
        if read_opt(&root.join(&f.path)) != f.before {
            bail!("{} changed after the preview was made; generate the preview again", f.path);
        }
    }
    for f in &cs.files {
        let path = root.join(&f.path);
        match &f.after {
            Some(content) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
                }
                std::fs::write(&path, content).with_context(|| format!("writing {}", f.path))?;
            }
            None => {
                if path.exists() {
                    std::fs::remove_file(&path).with_context(|| format!("removing {}", f.path))?;
                }
            }
        }
    }
    Ok(())
}

/// Reverts a changeset that was applied. Refuses if files were edited since.
pub fn undo(root: &Path, cs: &Changeset) -> Result<()> {
    for f in &cs.files {
        if read_opt(&root.join(&f.path)) != f.after {
            bail!("{} was edited after it was generated, so undo would lose changes", f.path);
        }
    }
    for f in &cs.files {
        let path = root.join(&f.path);
        match &f.before {
            Some(content) => std::fs::write(&path, content).with_context(|| format!("restoring {}", f.path))?,
            None => {
                if path.exists() {
                    std::fs::remove_file(&path).with_context(|| format!("removing {}", f.path))?;
                }
                // Remove directories the changeset created, if they are now empty.
                let mut dir = path.parent().map(Path::to_path_buf);
                while let Some(d) = dir {
                    if d == root || std::fs::remove_dir(&d).is_err() {
                        break;
                    }
                    dir = d.parent().map(Path::to_path_buf);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_and_undo_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "one\n").unwrap();
        let mut t = Tree::new(dir.path());
        t.write("a.ts", "one\ntwo\n");
        t.write("src/new/b.ts", "b\n");
        let cs = t.into_changeset("test", None);
        assert_eq!(cs.files.len(), 2);
        assert_eq!(cs.files[0].status, ChangeStatus::Modified);
        assert_eq!(cs.files[0].additions, 1);
        apply(dir.path(), &cs).unwrap();
        assert!(dir.path().join("src/new/b.ts").exists());
        undo(dir.path(), &cs).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("a.ts")).unwrap(), "one\n");
        assert!(!dir.path().join("src").exists());
    }

    #[test]
    fn refuses_stale_preview() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "one\n").unwrap();
        let mut t = Tree::new(dir.path());
        t.write("a.ts", "two\n");
        let cs = t.into_changeset("test", None);
        std::fs::write(dir.path().join("a.ts"), "edited\n").unwrap();
        assert!(apply(dir.path(), &cs).is_err());
    }
}
