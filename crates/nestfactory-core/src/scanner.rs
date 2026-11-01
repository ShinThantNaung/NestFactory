//! Walks a Nest project, parses TypeScript files in parallel and caches results by
//! modification time, so a rescan after a save only re-parses what changed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use rayon::prelude::*;

use crate::analyze::{analyze, FileEntry};
use crate::model::ProjectModel;
use crate::project::read_meta;
use crate::ts::parse::Parsed;

const SKIP_DIRS: &[&str] = &["node_modules", "dist", ".git", "coverage", "target", "build", ".nestfactory"];

#[derive(Default)]
pub struct Scanner {
    cache: HashMap<PathBuf, (SystemTime, u64, Arc<Parsed>)>,
}

impl Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn scan(&mut self, root: &Path) -> ProjectModel {
        let started = Instant::now();
        let meta = read_meta(root);
        let paths = collect_ts_files(root);
        let cache = &self.cache;
        let results: Vec<(PathBuf, String, SystemTime, u64, Arc<Parsed>)> = paths
            .par_iter()
            .filter_map(|(abs, rel)| {
                let md = std::fs::metadata(abs).ok()?;
                let modified = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let len = md.len();
                if let Some((m, l, p)) = cache.get(abs) {
                    if *m == modified && *l == len {
                        return Some((abs.clone(), rel.clone(), modified, len, p.clone()));
                    }
                }
                let src = std::fs::read_to_string(abs).ok()?;
                Some((abs.clone(), rel.clone(), modified, len, Arc::new(Parsed::new(src))))
            })
            .collect();
        let mut next_cache = HashMap::with_capacity(results.len());
        let mut entries = Vec::with_capacity(results.len());
        for (abs, rel, m, l, p) in results {
            next_cache.insert(abs, (m, l, p.clone()));
            entries.push(FileEntry { rel, parsed: p });
        }
        entries.sort_by(|a, b| a.rel.cmp(&b.rel));
        self.cache = next_cache;
        let mut model = analyze(&meta, &entries);
        model.scan_ms = started.elapsed().as_secs_f64() * 1000.0;
        model
    }

    /// Parsed content of a project file, from the cache when fresh.
    pub fn parsed(&self, abs: &Path) -> Option<Arc<Parsed>> {
        self.cache.get(abs).map(|(_, _, p)| p.clone())
    }
}

pub fn collect_ts_files(root: &Path) -> Vec<(PathBuf, String)> {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_some_and(|t| t.is_dir()) && SKIP_DIRS.contains(&name.as_ref()))
        })
        .build();
    let mut out = vec![];
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy();
        if !name.ends_with(".ts") || name.ends_with(".d.ts") {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push((path.to_path_buf(), rel.to_string_lossy().replace('\\', "/")));
        }
    }
    out
}
