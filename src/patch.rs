//! Streaming parser for `git log -p --unified=0` output in the format that
//! [`crate::git::log_args`] asks for.

use std::io::BufRead;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
    pub removed: Vec<Vec<u8>>,
    pub added: Vec<Vec<u8>>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct FileDiff {
    /// Path before the change; `None` for a new file.
    pub old: Option<Vec<u8>>,
    /// Path after the change; `None` for a deleted file.
    pub new: Option<Vec<u8>>,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub time: i64,
    /// Parent hashes, when the format asked for them (`%x01%H %ct %P`).
    pub parents: Vec<String>,
    pub author: String,
    pub committer: String,
    pub message: String,
    pub files: Vec<FileDiff>,
}

pub const START: u8 = 0x01;
pub const END_MSG: u8 = 0x02;

/// Parses `@@ -a,b +c,d @@` (counts default to 1).
pub fn parse_hunk_header(line: &[u8]) -> Option<(usize, usize, usize, usize)> {
    let s = std::str::from_utf8(line).ok()?;
    let s = s.strip_prefix("@@ -")?;
    let end = s.find(" @@")?;
    let (old, new) = s[..end].split_once(" +")?;
    let range = |r: &str| -> Option<(usize, usize)> {
        match r.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((r.parse().ok()?, 1)),
        }
    };
    let (a, b) = range(old)?;
    let (c, d) = range(new)?;
    Some((a, b, c, d))
}

fn strip_path(rest: &[u8], prefix: &[u8]) -> Option<Vec<u8>> {
    if rest == b"/dev/null" {
        return None;
    }
    let p = rest.strip_prefix(prefix).unwrap_or(rest);
    // git appends a tab when the path has trailing spaces
    let p = p.strip_suffix(b"\t").unwrap_or(p);
    Some(p.to_vec())
}

/// Reads commits one at a time and calls `f` for each.
pub fn parse<R: BufRead>(mut r: R, mut f: impl FnMut(Commit)) -> std::io::Result<()> {
    let mut line = Vec::with_capacity(256);
    let mut cur: Option<Commit> = None;
    #[derive(PartialEq)]
    enum St {
        Author,
        Committer,
        Message,
        Diff,
    }
    let mut st = St::Diff;
    loop {
        line.clear();
        if r.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') && st != St::Diff {
            line.pop();
        }
        if line.first() == Some(&START) {
            if let Some(c) = cur.take() {
                f(c);
            }
            let head = String::from_utf8_lossy(&line[1..]).to_string();
            let mut it = head.split(' ');
            let hash = it.next().unwrap_or_default().to_string();
            let time = it.next().and_then(|t| t.parse().ok()).unwrap_or(0);
            let parents = it.filter(|p| !p.is_empty()).map(|p| p.to_string()).collect();
            cur = Some(Commit { hash, time, parents, ..Default::default() });
            st = St::Author;
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        match st {
            St::Author => {
                c.author = String::from_utf8_lossy(&line).to_string();
                st = St::Committer;
            }
            St::Committer => {
                c.committer = String::from_utf8_lossy(&line).to_string();
                st = St::Message;
            }
            St::Message => {
                if let Some(pos) = line.iter().position(|&b| b == END_MSG) {
                    c.message.push_str(&String::from_utf8_lossy(&line[..pos]));
                    st = St::Diff;
                } else {
                    c.message.push_str(&String::from_utf8_lossy(&line));
                    c.message.push('\n');
                }
            }
            St::Diff => diff_line(c, &line),
        }
    }
    if let Some(c) = cur.take() {
        f(c);
    }
    Ok(())
}

fn diff_line(c: &mut Commit, line: &[u8]) {
    if line.starts_with(b"diff --git ") {
        c.files.push(FileDiff::default());
        // Fallback paths from the header; reliable when there are no spaces.
        // They are overwritten by ---/+++ or rename lines when present.
        let rest = &line[11..];
        if let Some(i) = find(rest, b" b/") {
            let (a, b) = (&rest[..i], &rest[i + 1..]);
            let fd = c.files.last_mut().unwrap();
            fd.old = Some(a.strip_prefix(b"a/").unwrap_or(a).to_vec());
            fd.new = Some(b.strip_prefix(b"b/").unwrap_or(b).to_vec());
        }
        return;
    }
    let Some(fd) = c.files.last_mut() else { return };
    if let Some(h) = fd.hunks.last_mut() {
        let in_hunk = h.removed.len() < h.old_len || h.added.len() < h.new_len;
        if in_hunk {
            match line.first() {
                Some(b'-') if h.removed.len() < h.old_len => {
                    h.removed.push(line[1..].to_vec());
                    return;
                }
                Some(b'+') if h.added.len() < h.new_len => {
                    h.added.push(line[1..].to_vec());
                    return;
                }
                Some(b'\\') => return, // "\ No newline at end of file"
                _ => {}
            }
        }
    }
    if line.starts_with(b"@@ ") {
        if let Some((a, b, cc, d)) = parse_hunk_header(line) {
            fd.hunks.push(Hunk { old_start: a, old_len: b, new_start: cc, new_len: d, ..Default::default() });
        }
    } else if let Some(p) = line.strip_prefix(b"--- ") {
        fd.old = strip_path(p, b"a/");
    } else if let Some(p) = line.strip_prefix(b"+++ ") {
        fd.new = strip_path(p, b"b/");
    } else if let Some(p) = line.strip_prefix(b"rename from ") {
        fd.old = Some(p.to_vec());
    } else if let Some(p) = line.strip_prefix(b"rename to ") {
        fd.new = Some(p.to_vec());
    } else if line.starts_with(b"new file mode") {
        fd.old = None;
    } else if line.starts_with(b"deleted file mode") {
        fd.new = None;
    } else if line.starts_with(b"Binary files ") || line.starts_with(b"GIT binary patch") {
        fd.binary = true;
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_headers() {
        assert_eq!(parse_hunk_header(b"@@ -1,3 +1,4 @@ fn x()"), Some((1, 3, 1, 4)));
        assert_eq!(parse_hunk_header(b"@@ -5 +5,0 @@"), Some((5, 1, 5, 0)));
        assert_eq!(parse_hunk_header(b"@@ -0,0 +1 @@"), Some((0, 0, 1, 1)));
        assert_eq!(parse_hunk_header(b"@@ nonsense"), None);
    }

    #[test]
    fn parses_commits_files_and_hunks() {
        let log = b"\x01abc 1700000000\nAda <ada@x.dev>\nAda <ada@x.dev>\nAdd things\n\nCo-authored-by: Claude <noreply@anthropic.com>\n\x02\n\ndiff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -2 +2,2 @@\n-old\n+new\n+--- not a header\n@@ -10,0 +12 @@\n+tail\ndiff --git a/x b/y\nsimilarity index 100%\nrename from old name.rs\nrename to new name.rs\ndiff --git a/bin.png b/bin.png\nnew file mode 100644\nBinary files /dev/null and b/bin.png differ\ndiff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n--- a/gone.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n\x01def 1700000100\nBob <b@x>\nBob <b@x>\nsecond\n\x02\n";
        let mut out = vec![];
        parse(&log[..], |c| out.push(c)).unwrap();
        assert_eq!(out.len(), 2);
        let c = &out[0];
        assert_eq!((c.hash.as_str(), c.time), ("abc", 1700000000));
        assert_eq!(c.author, "Ada <ada@x.dev>");
        assert!(c.message.contains("Co-authored-by: Claude"));
        assert_eq!(c.files.len(), 4);
        let a = &c.files[0];
        assert_eq!(a.new.as_deref(), Some(&b"src/a.rs"[..]));
        assert_eq!(a.hunks.len(), 2);
        assert_eq!(a.hunks[0].removed, vec![b"old".to_vec()]);
        assert_eq!(a.hunks[0].added, vec![b"new".to_vec(), b"--- not a header".to_vec()]);
        assert_eq!(a.hunks[1].added, vec![b"tail".to_vec()]);
        let r = &c.files[1];
        assert_eq!((r.old.as_deref(), r.new.as_deref()), (Some(&b"old name.rs"[..]), Some(&b"new name.rs"[..])));
        assert!(c.files[2].binary && c.files[2].old.is_none());
        assert!(c.files[3].new.is_none() && c.files[3].hunks[0].removed.len() == 2);
        assert_eq!(out[1].message, "second\n");
        assert!(out[0].parents.is_empty());
        assert!(out[1].files.is_empty());
    }
}
