//! Jest results and coverage, and TypeScript compiler diagnostics.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
    Todo,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestCase {
    pub title: String,
    pub ancestors: Vec<String>,
    pub full_name: String,
    pub status: TestStatus,
    pub duration_ms: f64,
    pub failure: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestSuite {
    pub file: String,
    pub e2e: bool,
    pub status: TestStatus,
    pub tests: Vec<TestCase>,
    /// Suite-level failure, e.g. a compile error before any test ran.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileCoverage {
    pub file: String,
    pub covered: Vec<u32>,
    pub uncovered: Vec<u32>,
    pub pct: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Coverage {
    pub lines_pct: f64,
    pub statements_pct: f64,
    pub files: Vec<FileCoverage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestRun {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub total: u32,
    pub duration_ms: f64,
    pub suites: Vec<TestSuite>,
    pub coverage: Option<Coverage>,
    pub error: Option<String>,
}

pub fn strip_ansi(s: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap()).replace_all(s, "").into_owned()
}

fn rel(root: &str, path: &str) -> String {
    let p = path.replace('\\', "/");
    let r = root.replace('\\', "/");
    let r = r.trim_end_matches('/');
    match p.to_lowercase().strip_prefix(&format!("{}/", r.to_lowercase())) {
        Some(_) => p[r.len() + 1..].to_string(),
        None => p,
    }
}

fn status(s: &str) -> TestStatus {
    match s {
        "passed" => TestStatus::Passed,
        "failed" => TestStatus::Failed,
        "todo" => TestStatus::Todo,
        _ => TestStatus::Skipped,
    }
}

/// Parses `jest --json --testLocationInResults` output.
pub fn parse_jest(root: &str, json: &str, e2e: bool) -> TestRun {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return TestRun { error: Some("Jest did not produce results".into()), ..Default::default() };
    };
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
    let mut run = TestRun {
        passed: n("numPassedTests"),
        failed: n("numFailedTests"),
        skipped: n("numPendingTests") + n("numTodoTests"),
        total: n("numTotalTests"),
        ..Default::default()
    };
    let start = v.get("startTime").and_then(Value::as_f64).unwrap_or(0.0);
    let mut end = start;
    for r in v.get("testResults").and_then(Value::as_array).into_iter().flatten() {
        let file = rel(root, r.get("name").and_then(Value::as_str).unwrap_or(""));
        end = end.max(r.get("endTime").and_then(Value::as_f64).unwrap_or(0.0));
        let tests: Vec<TestCase> = r
            .get("assertionResults")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|a| {
                let failures: Vec<String> = a.get("failureMessages").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(strip_ansi).collect();
                TestCase {
                    title: a.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                    ancestors: a.get("ancestorTitles").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(String::from).collect(),
                    full_name: a.get("fullName").and_then(Value::as_str).unwrap_or("").to_string(),
                    status: status(a.get("status").and_then(Value::as_str).unwrap_or("")),
                    duration_ms: a.get("duration").and_then(Value::as_f64).unwrap_or(0.0),
                    failure: if failures.is_empty() { None } else { Some(failures.join("\n")) },
                    line: a.pointer("/location/line").and_then(Value::as_u64).unwrap_or(0) as u32,
                }
            })
            .collect();
        let st = status(r.get("status").and_then(Value::as_str).unwrap_or(""));
        let message = r.get("message").and_then(Value::as_str).map(strip_ansi).filter(|m| !m.trim().is_empty() && tests.is_empty());
        run.suites.push(TestSuite { file, e2e, status: st, tests, message });
    }
    run.suites.sort_by(|a, b| a.file.cmp(&b.file));
    run.duration_ms = (end - start).max(0.0);
    run
}

/// Line coverage from Istanbul's `coverage-final.json`.
pub fn parse_coverage(root: &str, json: &str) -> Option<Coverage> {
    let v: Value = serde_json::from_str(json).ok()?;
    let mut files = vec![];
    let mut total_lines = 0usize;
    let mut covered_lines = 0usize;
    let mut total_st = 0usize;
    let mut covered_st = 0usize;
    for (path, data) in v.as_object()? {
        let mut lines: BTreeMap<u32, bool> = BTreeMap::new();
        let map = data.get("statementMap").and_then(Value::as_object);
        let counts = data.get("s").and_then(Value::as_object);
        if let (Some(map), Some(counts)) = (map, counts) {
            for (id, loc) in map {
                let line = loc.pointer("/start/line").and_then(Value::as_u64).unwrap_or(0) as u32;
                let hit = counts.get(id).and_then(Value::as_u64).unwrap_or(0) > 0;
                total_st += 1;
                if hit {
                    covered_st += 1;
                }
                let e = lines.entry(line).or_insert(false);
                *e |= hit;
            }
        }
        let covered: Vec<u32> = lines.iter().filter(|(_, h)| **h).map(|(l, _)| *l).collect();
        let uncovered: Vec<u32> = lines.iter().filter(|(_, h)| !**h).map(|(l, _)| *l).collect();
        total_lines += lines.len();
        covered_lines += covered.len();
        let pct = if lines.is_empty() { 100.0 } else { covered.len() as f64 * 100.0 / lines.len() as f64 };
        files.push(FileCoverage { file: rel(root, path), covered, uncovered, pct });
    }
    files.sort_by(|a, b| a.file.cmp(&b.file));
    let pct = |c: usize, t: usize| if t == 0 { 100.0 } else { (c as f64 * 1000.0 / t as f64).round() / 10.0 };
    Some(Coverage { lines_pct: pct(covered_lines, total_lines), statements_pct: pct(covered_st, total_st), files })
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Problem {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub code: String,
    pub message: String,
    pub severity: String,
}

/// Parses `tsc --noEmit --pretty false` output.
pub fn parse_tsc(root: &str, output: &str) -> Vec<Problem> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^(.+?)\((\d+),(\d+)\): (error|warning) (TS\d+): (.*)$").unwrap());
    let mut out: Vec<Problem> = vec![];
    for line in output.lines() {
        if let Some(c) = re.captures(line.trim_end()) {
            out.push(Problem {
                file: rel(root, &c[1]),
                line: c[2].parse().unwrap_or(0),
                column: c[3].parse().unwrap_or(0),
                code: c[5].to_string(),
                message: c[6].to_string(),
                severity: c[4].to_string(),
            });
        } else if line.starts_with("  ") {
            if let Some(last) = out.last_mut() {
                last.message.push('\n');
                last.message.push_str(line.trim());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jest_results() {
        let json = r#"{"numFailedTests":1,"numPassedTests":1,"numPendingTests":0,"numTodoTests":0,"numTotalTests":2,"startTime":1000,
        "testResults":[{"name":"C:\\p\\src\\orders\\orders.service.spec.ts","status":"failed","endTime":1500,"message":"","assertionResults":[
        {"ancestorTitles":["OrdersService"],"title":"creates an order","fullName":"OrdersService creates an order","status":"passed","duration":4,"failureMessages":[],"location":{"line":12,"column":3}},
        {"ancestorTitles":["OrdersService"],"title":"applies VIP discount","fullName":"OrdersService applies VIP discount","status":"failed","duration":2,"failureMessages":["\u001b[31mExpected: 90\u001b[39m"],"location":{"line":40,"column":3}}]}]}"#;
        let r = parse_jest("C:/p", json, false);
        assert_eq!(r.failed, 1);
        assert_eq!(r.suites[0].file, "src/orders/orders.service.spec.ts");
        assert_eq!(r.suites[0].tests[1].failure.as_deref(), Some("Expected: 90"));
        assert_eq!(r.suites[0].tests[1].line, 40);
        assert_eq!(r.duration_ms, 500.0);
    }

    #[test]
    fn coverage_lines() {
        let json = r#"{"C:/p/src/a.ts":{"statementMap":{"0":{"start":{"line":1}},"1":{"start":{"line":2}},"2":{"start":{"line":2}}},"s":{"0":1,"1":0,"2":3}},
        "C:/p/src/b.ts":{"statementMap":{"0":{"start":{"line":5}}},"s":{"0":0}}}"#;
        let c = parse_coverage("C:/p", json).unwrap();
        assert_eq!(c.files[0].covered, vec![1, 2]);
        assert_eq!(c.files[1].uncovered, vec![5]);
        assert_eq!(c.lines_pct, 66.7);
    }

    #[test]
    fn tsc_output() {
        let out = "src/orders/orders.service.ts(19,28): error TS2339: Property 'tier' does not exist on type 'User'.\n  Did you mean 'tiers'?\n";
        let p = parse_tsc("C:/p", out);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].line, 19);
        assert!(p[0].message.ends_with("Did you mean 'tiers'?"));
    }
}
