//! End-to-end: build a small repository with real git, run the binary, check the numbers.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str], date: &str) {
    let st = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .status()
        .unwrap();
    assert!(st.success(), "git {args:?}");
}

fn write(dir: &Path, file: &str, lines: &[&str]) {
    std::fs::write(dir.join(file), lines.join("\n") + "\n").unwrap();
}

fn temp_repo(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("aichurn-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    git(&d, &["init", "-q", "-b", "main"], "2026-01-01T00:00:00Z");
    git(&d, &["config", "core.autocrlf", "false"], "2026-01-01T00:00:00Z");
    d
}

fn run(dir: &Path, extra: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_aichurn")).arg(dir).args(["--since", "all", "--by", "month"]).args(extra).env("NO_COLOR", "1").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn measures_churn_for_ai_and_human_commits() {
    let d = temp_repo("churn");
    // Jan 1: an agent writes 10 lines
    let ai: Vec<String> = (0..10).map(|i| format!("let value_{i} = compute({i});")).collect();
    let ai_refs: Vec<&str> = ai.iter().map(|s| s.as_str()).collect();
    write(&d, "ai.rs", &ai_refs);
    git(&d, &["add", "."], "2026-01-01T10:00:00Z");
    git(&d, &["commit", "-q", "-m", "Add values\n\nCo-authored-by: Claude <noreply@anthropic.com>"], "2026-01-01T10:00:00Z");
    // Jan 1: a human writes 10 lines
    let hu: Vec<String> = (0..10).map(|i| format!("let human_{i} = handmade({i});")).collect();
    let hu_refs: Vec<&str> = hu.iter().map(|s| s.as_str()).collect();
    write(&d, "human.rs", &hu_refs);
    git(&d, &["add", "."], "2026-01-01T11:00:00Z");
    git(&d, &["commit", "-q", "-m", "Add human values"], "2026-01-01T11:00:00Z");
    // Jan 5: rewrite 5 AI lines and 1 human line (within 14 days)
    let mut ai2 = ai_refs.clone();
    for l in ai2.iter_mut().take(5) {
        *l = "let rewritten = again();";
    }
    write(&d, "ai.rs", &ai2);
    let mut hu2 = hu_refs.clone();
    hu2[9] = "let human_9 = fixed(9);";
    write(&d, "human.rs", &hu2);
    git(&d, &["commit", "-q", "-am", "Fix"], "2026-01-05T09:00:00Z");
    // Mar 1: a later commit so January lines have a full window
    write(&d, "later.rs", &["fn later_function() {}"]);
    git(&d, &["add", "."], "2026-03-01T09:00:00Z");
    git(&d, &["commit", "-q", "-m", "Later"], "2026-03-01T09:00:00Z");

    let json = run(&d, &["--json"]);
    // all: 10 + 10 + 6 new lines (Jan) ; churned 5 AI + 1 human
    assert!(json.contains(r#""period":"2026-01""#), "{json}");
    assert!(json.contains(r#""ai":{"commits":1,"new_lines":10,"eligible_lines":10,"churned_lines":5"#), "{json}");
    assert!(json.contains(r#""human":{"commits":2,"new_lines":16,"eligible_lines":16,"churned_lines":1"#), "{json}");
    assert!(json.contains(r#""agents":[{"name":"Claude Code","commits":1}]"#), "{json}");

    let text = run(&d, &[]);
    assert!(text.contains("aichurn"), "{text}");
    assert!(text.contains("2026-01"), "{text}");

    let md = run(&d, &["--markdown"]);
    assert!(md.contains("| **All** | 27 |"), "{md}");

    let svg = d.join("card.svg");
    let badge = d.join("badge.svg");
    run(&d, &["--card", svg.to_str().unwrap(), "--badge", badge.to_str().unwrap()]);
    assert!(std::fs::read_to_string(&svg).unwrap().starts_with("<svg"));
    assert!(std::fs::read_to_string(&badge).unwrap().contains("2-week churn"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn merged_branches_carry_their_agent() {
    let d = temp_repo("merge");
    write(&d, "a.rs", &["fn main_line() {}"]);
    git(&d, &["add", "."], "2026-01-01T10:00:00Z");
    git(&d, &["commit", "-q", "-m", "init"], "2026-01-01T10:00:00Z");
    git(&d, &["checkout", "-q", "-b", "feature"], "2026-01-01T10:00:00Z");
    write(&d, "b.rs", &["fn agent_work_one() {}", "fn agent_work_two() {}"]);
    git(&d, &["add", "."], "2026-01-02T10:00:00Z");
    git(&d, &["commit", "-q", "-m", "feature\n\nCo-authored-by: Cursor Agent <cursoragent@cursor.com>"], "2026-01-02T10:00:00Z");
    git(&d, &["checkout", "-q", "main"], "2026-01-03T10:00:00Z");
    git(&d, &["merge", "-q", "--no-ff", "-m", "Merge feature", "feature"], "2026-01-03T10:00:00Z");
    let json = run(&d, &["--json"]);
    assert!(json.contains(r#""agents":[{"name":"Cursor","commits":1}]"#), "{json}");
    assert!(json.contains(r#""ai":{"commits":1,"new_lines":2"#), "{json}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn rejects_bad_arguments() {
    let out = Command::new(env!("CARGO_BIN_EXE_aichurn")).args(["--by", "week"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = Command::new(env!("CARGO_BIN_EXE_aichurn")).arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("aichurn 0.1.0"));
}
