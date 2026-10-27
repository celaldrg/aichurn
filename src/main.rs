use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use aichurn::engine::{self, Config, Engine, Grain, DAY};
use aichurn::{agents, git, patch, render, report::Report};

const USAGE: &str = "aichurn: does AI-written code get rewritten faster?

Usage:
  aichurn [path | owner/repo | git-url] [options]

Replays the git history and measures, per quarter and for AI-assisted vs human
commits: two-week churn (new lines rewritten or deleted soon after), copy/paste
(new lines that repeat an existing 6-line block) and moved code.

Options:
  --since <when>          start of the analysis: a date (2025-01-01), an age
                          (90d, 18m, 2y) or \"all\" (default: 2y)
  --rev <rev>             branch, tag or commit to analyse (default: HEAD)
  --by <quarter|month>    period size (default: quarter)
  --window <days>         churn window in days (default: 14)
  --max-commit-lines <n>  don't count commits that change more lines (default: 5000)
  --all-files             also count docs, data, lockfiles and generated files
  --json                  print the report as JSON
  --markdown              print a Markdown summary
  --card <file.svg>       write a shareable SVG report card
  --badge <file.svg>      write a shields-style badge
  --badge-label <s>       badge label (default: \"2-week churn\")
  --fail-above <pct>      exit 1 if overall churn is above pct (CI gate)
  --no-color              disable colors (also respects NO_COLOR)
  --version               print the version
";

struct Opts {
    target: String,
    since: String,
    rev: String,
    grain: Grain,
    window: i64,
    max_commit_lines: usize,
    all_files: bool,
    json: bool,
    markdown: bool,
    card: Option<String>,
    badge: Option<String>,
    badge_label: String,
    fail_above: Option<f64>,
    no_color: bool,
}

fn parse_args(args: Vec<String>) -> Result<Option<Opts>, String> {
    let mut o = Opts {
        target: ".".into(),
        since: "2y".into(),
        rev: "HEAD".into(),
        grain: Grain::Quarter,
        window: 14,
        max_commit_lines: 5000,
        all_files: false,
        json: false,
        markdown: false,
        card: None,
        badge: None,
        badge_label: "2-week churn".into(),
        fail_above: None,
        no_color: false,
    };
    let mut positional = vec![];
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut val = |name: &str| -> Result<String, String> { inline.clone().or_else(|| it.next()).ok_or(format!("{name} needs a value")) };
        match flag.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("aichurn {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--since" => o.since = val("--since")?,
            "--rev" => o.rev = val("--rev")?,
            "--by" => {
                o.grain = match val("--by")?.as_str() {
                    "quarter" | "q" => Grain::Quarter,
                    "month" | "m" => Grain::Month,
                    x => return Err(format!("--by: expected quarter or month, got {x:?}")),
                }
            }
            "--window" => o.window = val("--window")?.parse().ok().filter(|d: &i64| *d >= 1 && *d <= 365).ok_or("--window: expected 1..365 days")?,
            "--max-commit-lines" => o.max_commit_lines = val("--max-commit-lines")?.parse().map_err(|_| "--max-commit-lines: expected a number")?,
            "--all-files" => o.all_files = true,
            "--json" => o.json = true,
            "--markdown" => o.markdown = true,
            "--card" => o.card = Some(val("--card")?),
            "--badge" => o.badge = Some(val("--badge")?),
            "--badge-label" => o.badge_label = val("--badge-label")?,
            "--fail-above" => o.fail_above = Some(val("--fail-above")?.trim_end_matches('%').parse().map_err(|_| "--fail-above: expected a percentage")?),
            "--no-color" => o.no_color = true,
            f if f.starts_with('-') && f.len() > 1 => return Err(format!("unknown option {f}")),
            _ => positional.push(a),
        }
    }
    if positional.len() > 1 {
        return Err(format!("expected one repository, got {}", positional.len()));
    }
    if let Some(t) = positional.pop() {
        o.target = t;
    }
    Ok(Some(o))
}

/// Parses --since into a unix time (None = all history).
pub fn parse_since(s: &str, now: i64) -> Result<Option<i64>, String> {
    let s = s.trim();
    if s == "all" {
        return Ok(None);
    }
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    if let Ok(n) = num.parse::<i64>() {
        let days = match unit {
            "d" => n,
            "w" => n * 7,
            "m" => n * 30,
            "y" => n * 365,
            _ => return Err(format!("--since: unknown unit in {s:?}")),
        };
        return Ok(Some(now - days * DAY));
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 3 {
        if let (Ok(y), Ok(m), Ok(d)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>(), parts[2].parse::<i64>()) {
            if (1..=12).contains(&m) && (1..=31).contains(&d) {
                return Ok(Some(engine::days_from_civil(y, m, d) * DAY));
            }
        }
    }
    Err(format!("--since: expected a date (2025-01-01), an age (90d, 18m, 2y) or \"all\", got {s:?}"))
}

fn signed_by(c: &patch::Commit) -> Option<usize> {
    let mut ids = vec![c.author.clone(), c.committer.clone()];
    ids.extend(agents::coauthors(&c.message));
    agents::detect(&ids, &c.message)
}

fn run(o: Opts) -> Result<ExitCode, String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let since = parse_since(&o.since, now)?;
    let interactive = std::io::stderr().is_terminal();
    // Remote repos are cloned only as far back as the analysis needs.
    let shallow = since.map(|t| engine::date(t - 2 * DAY));
    let repo = git::open(&o.target, shallow.as_deref(), !interactive)?;
    let head_time = git::commit_time(&repo.dir, &o.rev)?;
    let base = match since {
        Some(t) => git::base_commit(&repo.dir, &o.rev, &engine::date(t))?,
        None => None,
    };

    let mut e = Engine::new(Config { window_days: o.window, grain: o.grain, max_commit_lines: o.max_commit_lines, head_time });
    if let Some(b) = &base {
        if interactive {
            eprint!("\rreading the snapshot at {} …", &b[..b.len().min(10)]);
        }
        git::snapshot(&repo.dir, b, o.all_files, |path, lines| e.seed_file(path, &lines))?;
    }
    // Pass 1: who signed what, across merged branches too.
    if interactive {
        eprint!("\r\x1b[Kreading commit graph …");
    }
    let mut all: Vec<(patch::Commit, Option<usize>)> = vec![];
    let (mut gchild, gout) = git::log_stream(&repo.dir, &git::graph_args(base.as_deref(), &o.rev))?;
    patch::parse(gout, |c| {
        let a = signed_by(&c);
        all.push((c, a));
    })
    .map_err(|err| err.to_string())?;
    let _ = gchild.wait();
    let range = match &base {
        Some(b) => format!("{b}..{}", o.rev),
        None => o.rev.clone(),
    };
    let mainline: Vec<String> = git::git(&repo.dir, &["rev-list", "--first-parent", "--reverse", &range])?.lines().map(|s| s.to_string()).collect();
    let merged = aichurn::branches::merge_agents(&all, &mainline);
    drop(all);

    // Pass 2: replay the first-parent history with patches.
    let args = git::log_args(base.as_deref(), &o.rev, o.all_files);
    let (mut child, out) = git::log_stream(&repo.dir, &args)?;
    let mut n = 0u64;
    patch::parse(out, |c| {
        let agent = signed_by(&c).or_else(|| merged.get(&c.hash).copied());
        e.apply_with(&c, agent, agents::is_automation(&c.author));
        n += 1;
        if interactive && n % 200 == 0 {
            eprint!("\rreplaying commits … {n}          ");
        }
    })
    .map_err(|err| err.to_string())?;
    let status = child.wait().map_err(|err| err.to_string())?;
    if !status.success() {
        return Err(format!("git log failed: {}", git::read_stderr(&mut child).trim()));
    }
    if interactive {
        eprint!("\r\x1b[K");
    }

    if std::env::var_os("AICHURN_VERIFY").is_some() {
        // Developer check: the replayed line counts must match the files at rev.
        let (mut ok, mut bad, mut real) = (0, 0, std::collections::HashSet::new());
        git::snapshot(&repo.dir, &o.rev, o.all_files, |path, lines| {
            if e.tracked_len(&path) == Some(lines.len()) {
                ok += 1;
            } else {
                bad += 1;
                if bad <= 5 {
                    eprintln!("verify: {} has {} lines, replay has {:?}", String::from_utf8_lossy(&path), lines.len(), e.tracked_len(&path));
                }
            }
            real.insert(path);
        })?;
        let extra = e.tracked_paths().filter(|p| !real.contains(*p) && e.tracked_len(p) != Some(0)).count();
        eprintln!("verify: {ok} files match, {bad} differ, {extra} tracked files missing at {}", o.rev);
    }

    let r = Report::build(&repo.name, &e, o.window, o.grain);
    if let Some(f) = &o.card {
        std::fs::write(f, render::card(&r)).map_err(|err| format!("{f}: {err}"))?;
    }
    if let Some(f) = &o.badge {
        std::fs::write(f, render::badge(&o.badge_label, &render::badge_value(&r))).map_err(|err| format!("{f}: {err}"))?;
    }
    if o.json {
        print!("{}", render::json(&r));
    } else if o.markdown {
        print!("{}", render::markdown(&r));
    } else {
        let force = std::env::var("FORCE_COLOR").unwrap_or_default();
        let forced = !force.is_empty() && force != "0" && force != "false";
        let color = !o.no_color && (forced || (std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()));
        print!("{}", render::text(&r, color));
    }
    if let (Some(limit), Some(c)) = (o.fail_above, r.total.all.churn_rate()) {
        if c * 100.0 > limit {
            eprintln!("aichurn: churn {:.1}% is above --fail-above {limit}%", c * 100.0);
            return Ok(ExitCode::from(1));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(args) {
        Ok(None) => ExitCode::SUCCESS,
        Ok(Some(o)) => match run(o) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("aichurn: {e}");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("aichurn: {e}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_parsing() {
        let now = 1_800_000_000;
        assert_eq!(parse_since("all", now), Ok(None));
        assert_eq!(parse_since("90d", now), Ok(Some(now - 90 * DAY)));
        assert_eq!(parse_since("2y", now), Ok(Some(now - 730 * DAY)));
        assert_eq!(parse_since("2026-01-01", now), Ok(Some(1_767_225_600)));
        assert!(parse_since("yesterday", now).is_err());
    }

    #[test]
    fn args() {
        let o = parse_args(vec!["vercel/next.js".into(), "--by".into(), "month".into(), "--window=7".into(), "--json".into()]).unwrap().unwrap();
        assert_eq!(o.target, "vercel/next.js");
        assert_eq!(o.grain, Grain::Month);
        assert_eq!(o.window, 7);
        assert!(o.json);
        assert!(parse_args(vec!["--nope".into()]).is_err());
        assert!(parse_args(vec!["a".into(), "b".into()]).is_err());
        assert!(parse_args(vec!["--window".into(), "0".into()]).is_err());
    }
}
