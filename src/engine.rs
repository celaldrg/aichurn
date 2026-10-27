//! Line provenance and the metrics computed from it.
//!
//! Every tracked file is a vector with one entry per line: when the line was
//! written and whether the commit that wrote it was AI-assisted. Commits are
//! replayed oldest first; when a line is deleted or rewritten within the churn
//! window of its birth, it counts as churn for the period it was written in.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::patch::{Commit, FileDiff};

pub const DAY: i64 = 86_400;
/// Lines in a duplicated block.
pub const DUP_WINDOW: usize = 6;

const F_AI: u8 = 1; // written in an AI-assisted commit
const F_COUNTED: u8 = 2; // new code that counts toward the metrics

#[derive(Clone, Copy, Debug)]
struct Birth {
    t: i64,
    f: u8,
}
const OLD: Birth = Birth { t: i64::MIN, f: 0 };

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Grain {
    Month,
    Quarter,
}

/// Numbers for one side (all, AI-assisted or human) of one period.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Side {
    /// Non-blank added lines that are not moves.
    pub new_lines: u64,
    /// New lines old enough for their churn window to be complete.
    pub eligible: u64,
    /// Eligible lines deleted or rewritten within the window.
    pub churned: u64,
    /// New lines inside a duplicated block of `DUP_WINDOW` lines.
    pub duplicated: u64,
    /// Added lines that were moved from elsewhere in the same commit.
    pub moved: u64,
    pub commits: u64,
}

impl Side {
    pub fn churn_rate(&self) -> Option<f64> {
        (self.eligible > 0).then(|| self.churned as f64 / self.eligible as f64)
    }
    pub fn dup_rate(&self) -> Option<f64> {
        (self.new_lines > 0).then(|| self.duplicated as f64 / self.new_lines as f64)
    }
    pub fn moved_rate(&self) -> Option<f64> {
        let d = self.new_lines + self.moved;
        (d > 0).then(|| self.moved as f64 / d as f64)
    }
    fn add(&mut self, o: &Side) {
        self.new_lines += o.new_lines;
        self.eligible += o.eligible;
        self.churned += o.churned;
        self.duplicated += o.duplicated;
        self.moved += o.moved;
        self.commits += o.commits;
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Period {
    pub all: Side,
    pub ai: Side,
    pub human: Side,
}

impl Period {
    fn side(&mut self, ai: bool) -> &mut Side {
        if ai {
            &mut self.ai
        } else {
            &mut self.human
        }
    }
    pub fn merged(&self) -> Period {
        let mut p = self.clone();
        p.all = Side::default();
        p.all.add(&self.ai);
        p.all.add(&self.human);
        p
    }
}

pub struct Config {
    pub window_days: i64,
    pub grain: Grain,
    /// Commits that change more lines than this are replayed but not counted
    /// (mass reformatting, vendoring, generated code).
    pub max_commit_lines: usize,
    /// Commit time of the newest commit; lines younger than the window are
    /// not yet eligible for churn.
    pub head_time: i64,
}

pub struct Engine {
    cfg: Config,
    files: HashMap<Vec<u8>, Vec<Birth>>,
    seen_windows: HashSet<u64>,
    pub periods: BTreeMap<String, Period>,
    pub agents: BTreeMap<&'static str, u64>,
    pub commits: u64,
    pub skipped_bulk: u64,
    pub skipped_bots: u64,
    pub first_time: Option<i64>,
    pub last_time: Option<i64>,
}

/// FNV-1a, stable across runs and platforms.
fn fnv(bytes: &[u8], seed: u64) -> u64 {
    let mut h = seed ^ 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Collapses whitespace. Returns `None` for lines that carry no meaning on
/// their own: blank lines, lone braces, and comment-only lines (license
/// headers would otherwise count as copy/paste).
pub fn meaningful(line: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(line.len());
    let mut space = false;
    for &b in line {
        if b.is_ascii_whitespace() {
            space = !out.is_empty();
        } else {
            if space {
                out.push(b' ');
                space = false;
            }
            out.push(b);
        }
    }
    let alnum = out.iter().filter(|b| b.is_ascii_alphanumeric()).count();
    if alnum < 4 {
        return None;
    }
    const COMMENTS: [&[u8]; 8] = [b"//", b"#", b"/*", b"*", b"<!--", b"--", b";", b"\"\"\""];
    if COMMENTS.iter().any(|c| out.starts_with(c)) {
        return None;
    }
    Some(out)
}

pub fn is_blank(line: &[u8]) -> bool {
    line.iter().all(|b| b.is_ascii_whitespace())
}

/// Days since 1970-01-01 to (year, month), proleptic Gregorian.
pub fn year_month(t: i64) -> (i64, u32) {
    let z = t.div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m)
}

pub fn date(t: i64) -> String {
    let z = t.div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix time at which a period ("2026-Q1" or "2026-03") ends.
pub fn period_end(key: &str) -> Option<i64> {
    let (y, rest) = key.split_once('-')?;
    let y: i64 = y.parse().ok()?;
    let last_month: i64 = match rest.strip_prefix('Q') {
        Some(q) => q.parse::<i64>().ok()? * 3,
        None => rest.parse().ok()?,
    };
    let (ny, nm) = if last_month == 12 { (y + 1, 1) } else { (y, last_month + 1) };
    Some(days_from_civil(ny, nm, 1) * DAY)
}

pub fn period_key(t: i64, g: Grain) -> String {
    let (y, m) = year_month(t);
    match g {
        Grain::Month => format!("{y}-{m:02}"),
        Grain::Quarter => format!("{y}-Q{}", (m - 1) / 3 + 1),
    }
}

impl Engine {
    pub fn new(cfg: Config) -> Self {
        Engine {
            cfg,
            files: HashMap::new(),
            seen_windows: HashSet::new(),
            periods: BTreeMap::new(),
            agents: BTreeMap::new(),
            commits: 0,
            skipped_bulk: 0,
            skipped_bots: 0,
            first_time: None,
            last_time: None,
        }
    }

    /// Registers a file that exists before the first replayed commit, so its
    /// lines are known (as old code) and its blocks can be matched as copies.
    pub fn seed_file(&mut self, path: Vec<u8>, lines: &[Vec<u8>]) {
        self.index_windows(lines.iter().map(|l| meaningful(l)).collect::<Vec<_>>().as_slice(), None);
        self.files.insert(path, vec![OLD; lines.len()]);
    }

    fn window_hashes(norm: &[Option<Vec<u8>>]) -> Vec<(usize, u64)> {
        // windows over consecutive meaningful lines; returns (index of last line, hash)
        let idx: Vec<usize> = (0..norm.len()).filter(|&i| norm[i].is_some()).collect();
        let mut out = vec![];
        if idx.len() < DUP_WINDOW {
            return out;
        }
        for w in idx.windows(DUP_WINDOW) {
            let mut h = 0u64;
            for &i in w {
                h = fnv(norm[i].as_ref().unwrap(), h.rotate_left(5));
            }
            out.push((w[DUP_WINDOW - 1], h));
        }
        out
    }

    /// Adds the windows of `norm` to the index. When `dup` is given, marks the
    /// lines of every window that was already in the index.
    fn index_windows(&mut self, norm: &[Option<Vec<u8>>], mut dup: Option<&mut Vec<bool>>) {
        let idx: Vec<usize> = (0..norm.len()).filter(|&i| norm[i].is_some()).collect();
        for (k, (_, h)) in Self::window_hashes(norm).into_iter().enumerate() {
            let fresh = self.seen_windows.insert(h);
            if !fresh {
                if let Some(d) = dup.as_deref_mut() {
                    for &i in &idx[k..k + DUP_WINDOW] {
                        d[i] = true;
                    }
                }
            }
        }
    }

    /// Replays one commit (oldest first).
    pub fn apply(&mut self, c: &Commit, agent: Option<usize>) {
        self.apply_with(c, agent, false)
    }

    /// Like [`Engine::apply`]; `automation` commits (bots) are replayed but not counted.
    pub fn apply_with(&mut self, c: &Commit, agent: Option<usize>, automation: bool) {
        let t = c.time;
        let ai = agent.is_some();
        self.commits += 1;
        self.first_time = Some(self.first_time.map_or(t, |x| x.min(t)));
        self.last_time = Some(self.last_time.map_or(t, |x| x.max(t)));
        let changed: usize = c.files.iter().flat_map(|f| f.hunks.iter()).map(|h| h.added.len() + h.removed.len()).sum();
        let counted = changed <= self.cfg.max_commit_lines && !(automation && agent.is_none());
        if automation && agent.is_none() {
            self.skipped_bots += 1;
        } else if !counted {
            self.skipped_bulk += 1;
        }
        let key = period_key(t, self.cfg.grain);
        if counted {
            let p = self.periods.entry(key.clone()).or_default();
            p.side(ai).commits += 1;
            if let Some(i) = agent {
                *self.agents.entry(crate::agents::AGENTS[i].name).or_default() += 1;
            }
        }

        // Lines deleted anywhere in this commit, to recognise moves.
        let mut deleted: HashMap<Vec<u8>, u32> = HashMap::new();
        if counted {
            for f in &c.files {
                for h in &f.hunks {
                    for l in &h.removed {
                        if let Some(n) = meaningful(l) {
                            *deleted.entry(n).or_default() += 1;
                        }
                    }
                }
            }
        }

        for f in &c.files {
            self.apply_file(f, t, ai, counted, &key, &mut deleted);
        }
    }

    fn apply_file(&mut self, f: &FileDiff, t: i64, ai: bool, counted: bool, key: &str, deleted: &mut HashMap<Vec<u8>, u32>) {
        if f.binary && f.hunks.is_empty() {
            match (&f.old, &f.new) {
                (Some(o), None) => {
                    self.files.remove(o);
                }
                (Some(o), Some(n)) if o != n => {
                    if let Some(v) = self.files.remove(o) {
                        self.files.insert(n.clone(), v);
                    }
                }
                _ => {}
            }
            return;
        }
        let mut lines = match (&f.old, &f.new) {
            (Some(o), _) => self.files.remove(o).unwrap_or_default(),
            (None, _) => Vec::new(),
        };
        let window = self.cfg.window_days * DAY;
        let eligible_before = self.cfg.head_time - window;
        let mut offset: isize = 0;
        let mut stats = Side::default();
        let mut churn_by_period: Vec<(String, bool)> = vec![];

        for h in &f.hunks {
            // position in the current (partially edited) vector
            let start = if h.old_len == 0 { h.old_start } else { h.old_start.saturating_sub(1) };
            let pos = (start as isize + offset).max(0) as usize;
            if lines.len() < pos + h.old_len {
                lines.resize(pos + h.old_len, OLD);
            }
            let removed: Vec<Birth> = lines.splice(pos..pos + h.old_len, std::iter::empty()).collect();
            for b in removed {
                if counted && b.f & F_COUNTED != 0 && b.t <= eligible_before && t - b.t <= window {
                    churn_by_period.push((period_key(b.t, self.cfg.grain), b.f & F_AI != 0));
                }
            }

            // classify added lines
            let norm: Vec<Option<Vec<u8>>> = h.added.iter().map(|l| meaningful(l)).collect();
            let mut dup = vec![false; norm.len()];
            self.index_windows(&norm, if counted { Some(&mut dup) } else { None });
            let mut births = Vec::with_capacity(h.added.len());
            for (i, l) in h.added.iter().enumerate() {
                let mut moved = false;
                if counted {
                    if let Some(n) = &norm[i] {
                        if let Some(c) = deleted.get_mut(n) {
                            if *c > 0 {
                                *c -= 1;
                                moved = true;
                            }
                        }
                    }
                }
                let blank = is_blank(l);
                let mut fl = if ai { F_AI } else { 0 };
                if counted && !blank && !moved {
                    fl |= F_COUNTED;
                    stats.new_lines += 1;
                    if t <= eligible_before {
                        stats.eligible += 1;
                    }
                    if dup[i] {
                        stats.duplicated += 1;
                    }
                } else if counted && moved {
                    stats.moved += 1;
                }
                births.push(Birth { t, f: fl });
            }
            lines.splice(pos..pos, births);
            offset += h.new_len as isize - h.old_len as isize;
        }

        if counted {
            let p = self.periods.entry(key.to_string()).or_default();
            let s = p.side(ai);
            s.new_lines += stats.new_lines;
            s.eligible += stats.eligible;
            s.duplicated += stats.duplicated;
            s.moved += stats.moved;
        }
        for (k, was_ai) in churn_by_period {
            self.periods.entry(k).or_default().side(was_ai).churned += 1;
        }
        if let Some(n) = &f.new {
            self.files.insert(n.clone(), lines);
        }
    }

    /// Periods with `all` filled in, oldest first.
    pub fn result(&self) -> Vec<(String, Period)> {
        self.periods.iter().map(|(k, p)| (k.clone(), p.merged())).collect()
    }

    /// Number of lines tracked for `path`, for consistency checks.
    pub fn tracked_len(&self, path: &[u8]) -> Option<usize> {
        self.files.get(path).map(|v| v.len())
    }

    pub fn tracked_paths(&self) -> impl Iterator<Item = &Vec<u8>> {
        self.files.keys()
    }

    pub fn head_time(&self) -> i64 {
        self.cfg.head_time
    }

    pub fn tracked_files(&self) -> usize {
        self.files.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::Hunk;

    fn cfg(head: i64) -> Config {
        Config { window_days: 14, grain: Grain::Month, max_commit_lines: 1000, head_time: head }
    }
    fn hunk(old_start: usize, removed: &[&str], new_start: usize, added: &[&str]) -> Hunk {
        Hunk {
            old_start,
            old_len: removed.len(),
            new_start,
            new_len: added.len(),
            removed: removed.iter().map(|s| s.as_bytes().to_vec()).collect(),
            added: added.iter().map(|s| s.as_bytes().to_vec()).collect(),
        }
    }
    fn commit(t: i64, files: Vec<FileDiff>) -> Commit {
        Commit { hash: "x".into(), time: t, files, ..Default::default() }
    }
    fn file(path: &str, new_file: bool, hunks: Vec<Hunk>) -> FileDiff {
        FileDiff { old: (!new_file).then(|| path.as_bytes().to_vec()), new: Some(path.as_bytes().to_vec()), binary: false, hunks }
    }

    const T0: i64 = 1_767_225_600; // 2026-01-01

    #[test]
    fn dates() {
        assert_eq!(date(T0), "2026-01-01");
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(period_key(T0, Grain::Quarter), "2026-Q1");
        assert_eq!(period_key(T0 + 200 * DAY, Grain::Month), "2026-07");
        assert_eq!(period_key(T0 + 200 * DAY, Grain::Quarter), "2026-Q3");
        assert_eq!(year_month(951_782_400), (2000, 2)); // 2000-02-29
        assert_eq!(period_end("2025-Q4"), Some(T0));
        assert_eq!(period_end("2025-12"), Some(T0));
        assert_eq!(period_end("2026-01"), Some(T0 + 31 * DAY));
    }

    #[test]
    fn churn_within_window_is_counted_for_the_birth_period() {
        let mut e = Engine::new(cfg(T0 + 90 * DAY));
        let lines = ["let alpha = 1;", "let beta = 2;", "let gamma = 3;", "let delta = 4;"];
        e.apply(&commit(T0, vec![file("a.rs", true, vec![hunk(0, &[], 1, &lines)])]), Some(0));
        // 3 days later a human rewrites line 2
        e.apply(&commit(T0 + 3 * DAY, vec![file("a.rs", false, vec![hunk(2, &["let beta = 2;"], 2, &["let beta = 20;"])])]), None);
        // 40 days later line 4 is deleted: outside the window, not churn
        e.apply(&commit(T0 + 40 * DAY, vec![file("a.rs", false, vec![hunk(4, &["let delta = 4;"], 3, &[])])]), None);
        let r = e.result();
        let jan = &r.iter().find(|(k, _)| k == "2026-01").unwrap().1;
        assert_eq!(jan.ai.new_lines, 4);
        assert_eq!(jan.ai.eligible, 4);
        assert_eq!(jan.ai.churned, 1);
        assert_eq!(jan.human.new_lines, 1);
        assert_eq!(jan.human.churned, 0);
        assert_eq!(jan.all.churned, 1);
        assert_eq!(jan.all.churn_rate(), Some(1.0 / 5.0));
        assert_eq!(e.agents.get("Claude Code"), Some(&1));
    }

    #[test]
    fn hunks_apply_with_offsets() {
        let mut e = Engine::new(cfg(T0 + 90 * DAY));
        e.apply(&commit(T0, vec![file("a", true, vec![hunk(0, &[], 1, &["l1 aaaa", "l2 bbbb", "l3 cccc", "l4 dddd", "l5 eeee"])])]), None);
        // insert after line 1, replace line 4 (positions refer to the old file)
        e.apply(&commit(T0 + DAY, vec![file("a", false, vec![hunk(1, &[], 2, &["new1 xxxx", "new2 yyyy"]), hunk(4, &["l4 dddd"], 6, &["l4 changed"])])]), None);
        assert_eq!(e.files[&b"a".to_vec()].len(), 7);
        let r = e.result();
        assert_eq!(r[0].1.all.churned, 1); // only l4 died young
    }

    #[test]
    fn young_lines_are_not_eligible_and_moves_are_not_new() {
        let head = T0 + 10 * DAY;
        let mut e = Engine::new(cfg(head));
        e.apply(&commit(T0, vec![file("a", true, vec![hunk(0, &[], 1, &["fn moved_function() {}", "other line here"])])]), None);
        // move the function to another file the next day
        e.apply(
            &commit(
                T0 + DAY,
                vec![file("a", false, vec![hunk(1, &["fn moved_function() {}"], 0, &[])]), file("b", true, vec![hunk(0, &[], 1, &["fn moved_function() {}"])])],
            ),
            None,
        );
        let p = &e.result()[0].1;
        assert_eq!(p.all.new_lines, 2);
        assert_eq!(p.all.moved, 1);
        assert_eq!(p.all.eligible, 0, "head is only 10 days after; nothing has a full 14-day window");
        assert_eq!(p.all.churned, 0);
    }

    #[test]
    fn duplicated_blocks_are_found() {
        let mut e = Engine::new(cfg(T0 + 90 * DAY));
        let block = ["let a = compute(1);", "let b = compute(2);", "let c = compute(3);", "let d = compute(4);", "let e = compute(5);", "let f = compute(6);"];
        e.seed_file(b"old.rs".to_vec(), &block.iter().map(|s| s.as_bytes().to_vec()).collect::<Vec<_>>());
        let mut added: Vec<&str> = vec!["fn fresh_code() {"];
        added.extend(block);
        e.apply(&commit(T0, vec![file("new.rs", true, vec![hunk(0, &[], 1, &added)])]), None);
        let p = &e.result()[0].1;
        assert_eq!(p.all.new_lines, 7);
        assert_eq!(p.all.duplicated, 6);
    }

    #[test]
    fn renames_keep_history_and_deletes_drop_it() {
        let mut e = Engine::new(cfg(T0 + 90 * DAY));
        e.apply(&commit(T0, vec![file("a", true, vec![hunk(0, &[], 1, &["line one here", "line two here"])])]), None);
        e.apply(&commit(T0 + DAY, vec![FileDiff { old: Some(b"a".to_vec()), new: Some(b"b".to_vec()), binary: false, hunks: vec![] }]), None);
        e.apply(&commit(T0 + 2 * DAY, vec![file("b", false, vec![hunk(1, &["line one here"], 0, &[])])]), None);
        assert_eq!(e.result()[0].1.all.churned, 1);
        e.apply(
            &commit(T0 + 3 * DAY, vec![FileDiff { old: Some(b"b".to_vec()), new: None, binary: false, hunks: vec![hunk(1, &["line two here"], 0, &[])] }]),
            None,
        );
        assert_eq!(e.tracked_files(), 0);
        assert_eq!(e.result()[0].1.all.churned, 2);
    }

    #[test]
    fn bulk_commits_are_replayed_but_not_counted() {
        let mut c = cfg(T0 + 90 * DAY);
        c.max_commit_lines = 3;
        let mut e = Engine::new(c);
        e.apply(&commit(T0, vec![file("a", true, vec![hunk(0, &[], 1, &["aaaa", "bbbb", "cccc", "dddd"])])]), None);
        assert_eq!(e.skipped_bulk, 1);
        assert!(e.result().is_empty());
        assert_eq!(e.files[&b"a".to_vec()].len(), 4);
    }

    #[test]
    fn meaningful_lines() {
        assert_eq!(meaningful(b"   let   x =  compute(); "), Some(b"let x = compute();".to_vec()));
        assert_eq!(meaningful(b"  }"), None);
        assert_eq!(meaningful(b"// Licensed under the MIT License"), None);
        assert_eq!(meaningful(b"# comment here"), None);
        assert!(is_blank(b"  \t"));
    }
}
