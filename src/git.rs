//! Running git: resolving the target, the starting snapshot and the log stream.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Paths that are not hand-written code: lockfiles, vendored and generated
/// files, data, docs and snapshots. Passed to git as exclude pathspecs.
pub const EXCLUDES: &[&str] = &[
    "**/*.lock",
    "**/package-lock.json",
    "**/pnpm-lock.yaml",
    "**/go.sum",
    "**/bun.lockb",
    "**/npm-shrinkwrap.json",
    "**/node_modules/**",
    "**/vendor/**",
    "**/third_party/**",
    "**/third-party/**",
    "**/dist/**",
    "**/build/**",
    "**/.yarn/**",
    "**/__snapshots__/**",
    "**/*.snap",
    "**/*.min.js",
    "**/*.min.css",
    "**/*.map",
    "**/*.svg",
    "**/*.json",
    "**/*.csv",
    "**/*.tsv",
    "**/*.ipynb",
    "**/*.md",
    "**/*.mdx",
    "**/*.rst",
    "**/*.txt",
    "**/*.pb.go",
    "**/*_pb2.py",
    "**/*.generated.*",
    "**/*_generated.*",
    "**/*.g.dart",
    "**/*.po",
    "**/*.pot",
    "**/*.xlf",
    "**/*.resx",
    "**/*.sum",
];

pub fn pathspecs(include_all: bool) -> Vec<String> {
    let mut v = vec!["--".to_string(), ".".to_string()];
    if !include_all {
        v.extend(EXCLUDES.iter().map(|e| format!(":(exclude,glob){e}")));
    }
    v
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).stdin(Stdio::null()).output().map_err(|e| format!("could not run git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A repository on disk, possibly a temporary clone that is removed on drop.
pub struct Repo {
    pub dir: PathBuf,
    pub name: String,
    temp: bool,
}

impl Drop for Repo {
    fn drop(&mut self) {
        if self.temp {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// `owner/repo`, an https/ssh URL, or a local path.
pub fn is_remote(target: &str) -> bool {
    if Path::new(target).exists() {
        return false;
    }
    target.starts_with("https://") || target.starts_with("http://") || target.starts_with("git@") || {
        let parts: Vec<&str> = target.split('/').collect();
        parts.len() == 2 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
    }
}

pub fn open(target: &str, shallow_since: Option<&str>, quiet: bool) -> Result<Repo, String> {
    if !is_remote(target) {
        let dir = PathBuf::from(target);
        let top = git(&dir, &["rev-parse", "--show-toplevel"]).or_else(|_| git(&dir, &["rev-parse", "--git-dir"]).map(|_| target.to_string()))?;
        let name = Path::new(&top).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| top.clone());
        return Ok(Repo { dir: PathBuf::from(top), name, temp: false });
    }
    let url = if target.contains("://") || target.starts_with("git@") { target.to_string() } else { format!("https://github.com/{target}.git") };
    let name = url.trim_end_matches(".git").trim_start_matches("https://github.com/").trim_start_matches("git@github.com:").to_string();
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("aichurn-{}-{nanos}", std::process::id()));
    if !quiet {
        eprintln!("cloning {url} …");
    }
    let clone = |shallow: Option<&str>| -> Result<bool, String> {
        let _ = std::fs::remove_dir_all(&dir);
        let mut cmd = Command::new("git");
        cmd.args(["clone", "--bare", "--single-branch", "--quiet"]);
        if let Some(d) = shallow {
            cmd.arg(format!("--shallow-since={d}"));
        }
        cmd.arg(&url).arg(&dir).stdin(Stdio::null()).stderr(Stdio::null());
        Ok(cmd.status().map_err(|e| format!("could not run git: {e}"))?.success())
    };
    let repo = Repo { dir: dir.clone(), name, temp: true };
    // A shallow clone fails when nothing is newer than the date; fall back to a full one.
    let ok = (shallow_since.is_some() && clone(shallow_since)?) || clone(None)?;
    if !ok {
        return Err(format!("git clone {url} failed (does the repository exist and is it public?)"));
    }
    Ok(repo)
}

pub fn commit_time(dir: &Path, rev: &str) -> Result<i64, String> {
    git(dir, &["log", "-1", "--format=%ct", rev, "--"])?.parse().map_err(|_| format!("no commits at {rev}"))
}

/// The newest first-parent commit at or before `since`, or the oldest
/// available commit when the history (or a shallow clone) starts later.
pub fn base_commit(dir: &Path, rev: &str, since: &str) -> Result<Option<String>, String> {
    let b = git(dir, &["rev-list", "-1", "--first-parent", &format!("--before={since}"), rev])?;
    if !b.is_empty() {
        return Ok(Some(b));
    }
    let shallow = git(dir, &["rev-parse", "--is-shallow-repository"]).unwrap_or_default() == "true";
    if shallow {
        let roots = git(dir, &["rev-list", "--first-parent", "--max-parents=0", rev])?;
        return Ok(roots.lines().last().map(|s| s.to_string()));
    }
    Ok(None)
}

/// Streams every text file of `rev` (respecting the excludes) as (path, lines).
pub fn snapshot(dir: &Path, rev: &str, include_all: bool, mut f: impl FnMut(Vec<u8>, Vec<Vec<u8>>)) -> Result<(), String> {
    let mut args: Vec<String> =
        ["-c", "core.quotePath=false", "grep", "-I", "--null", "-n", "--no-color", "-e", "", rev].iter().map(|s| s.to_string()).collect();
    args.extend(pathspecs(include_all));
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run git: {e}"))?;
    let prefix = format!("{rev}:").into_bytes();
    let mut r = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());
    let mut buf = Vec::new();
    let mut cur: Option<(Vec<u8>, Vec<Vec<u8>>)> = None;
    loop {
        buf.clear();
        if r.read_until(b'\n', &mut buf).map_err(|e| e.to_string())? == 0 {
            break;
        }
        if buf.last() == Some(&b'\n') {
            buf.pop();
        }
        // REV:path \0 lineno \0 content
        let mut parts = buf.splitn(3, |&b| b == 0);
        let (Some(p), Some(_), Some(content)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let path = p.strip_prefix(prefix.as_slice()).unwrap_or(p);
        match &mut cur {
            Some((cp, lines)) if cp.as_slice() == path => lines.push(content.to_vec()),
            _ => {
                if let Some((cp, lines)) = cur.take() {
                    f(cp, lines);
                }
                cur = Some((path.to_vec(), vec![content.to_vec()]));
            }
        }
    }
    if let Some((cp, lines)) = cur.take() {
        f(cp, lines);
    }
    let _ = child.wait();
    Ok(())
}

/// Arguments for the replay log: first-parent history, oldest first, with
/// zero-context patches and rename detection.
pub fn log_args(base: Option<&str>, rev: &str, include_all: bool) -> Vec<String> {
    let range = match base {
        Some(b) => format!("{b}..{rev}"),
        None => rev.to_string(),
    };
    let mut v: Vec<String> = [
        "-c",
        "core.quotePath=false",
        "-c",
        "diff.renameLimit=3000",
        "log",
        "--first-parent",
        "--diff-merges=first-parent",
        "--reverse",
        "-p",
        "--unified=0",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "-M",
        "--format=%x01%H %ct%n%an <%ae>%n%cn <%ce>%n%B%x02",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    v.push(range);
    v.extend(pathspecs(include_all));
    v
}

/// Arguments for the attribution pass: every commit in the range with its
/// parents and signatures, no patches.
pub fn graph_args(base: Option<&str>, rev: &str) -> Vec<String> {
    let range = match base {
        Some(b) => format!("{b}..{rev}"),
        None => rev.to_string(),
    };
    ["log", "--no-color", "--format=%x01%H %ct %P%n%an <%ae>%n%cn <%ce>%n%B%x02", &range].iter().map(|s| s.to_string()).collect()
}

pub fn log_stream(dir: &Path, args: &[String]) -> Result<(std::process::Child, BufReader<std::process::ChildStdout>), String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run git: {e}"))?;
    let out = BufReader::with_capacity(1 << 20, child.stdout.take().unwrap());
    Ok((child, out))
}

pub fn read_stderr(child: &mut std::process::Child) -> String {
    let mut s = String::new();
    if let Some(e) = child.stderr.take() {
        BufReader::new(e).lines().map_while(Result::ok).for_each(|l| {
            s.push_str(&l);
            s.push('\n');
        });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_detection() {
        assert!(is_remote("vercel/next.js"));
        assert!(is_remote("https://github.com/a/b"));
        assert!(is_remote("git@github.com:a/b.git"));
        assert!(!is_remote("."));
        assert!(!is_remote("some/deep/path"));
    }
}
