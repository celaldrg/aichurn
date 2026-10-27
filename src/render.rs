//! Terminal, Markdown, JSON and SVG output.

use std::fmt::Write as _;

use crate::engine::{Grain, Side};
use crate::report::{pct, side_or_dash, thousands, Report, MIN_AI_LINES};

struct Pal(bool);
impl Pal {
    fn rgb(&self, (r, g, b): (u8, u8, u8), s: &str) -> String {
        if self.0 {
            format!("\x1b[38;2;{r};{g};{b}m{s}\x1b[39m")
        } else {
            s.to_string()
        }
    }
    fn bold(&self, s: &str) -> String {
        if self.0 {
            format!("\x1b[1m{s}\x1b[22m")
        } else {
            s.to_string()
        }
    }
    fn dim(&self, s: &str) -> String {
        if self.0 {
            format!("\x1b[2m{s}\x1b[22m")
        } else {
            s.to_string()
        }
    }
}
const ACCENT: (u8, u8, u8) = (45, 212, 191);
const AI: (u8, u8, u8) = (251, 146, 60);
const BAD: (u8, u8, u8) = (248, 81, 73);
const GOOD: (u8, u8, u8) = (63, 185, 80);

fn pad(s: &str, w: usize, right: bool) -> String {
    let n = s.chars().count();
    if n >= w {
        return s.to_string();
    }
    let sp = " ".repeat(w - n);
    if right {
        sp + s
    } else {
        s.to_string() + &sp
    }
}

pub fn sparkline(vals: &[f64]) -> String {
    const S: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = vals.iter().cloned().fold(0.0, f64::max);
    vals.iter().map(|v| if max > 0.0 { S[((v / max) * 7.0).round() as usize] } else { S[0] }).collect()
}

/// One sentence for headlines, alt text and the card.
pub fn headline(r: &Report) -> String {
    let Some(c) = r.total.all.churn_rate() else {
        return "Not enough history to measure churn yet.".into();
    };
    let mut s = format!("{} of new lines were rewritten or deleted within {} days.", pct(c), r.window_days);
    if let Some(x) = r.ai_ratio() {
        s += &format!(" {}", ai_verdict(r, x));
    }
    s
}

pub fn ai_verdict(r: &Report, x: f64) -> String {
    let a = side_or_dash(&r.total.ai, Side::churn_rate);
    let h = side_or_dash(&r.total.human, Side::churn_rate);
    if (x - 1.0).abs() < 0.1 {
        format!("Lines from AI-assisted commits churned about as often as human lines ({a} vs {h}).")
    } else if x > 1.0 {
        format!("Lines from AI-assisted commits churned {x:.1}× as often as human lines ({a} vs {h}).")
    } else {
        format!("Lines from AI-assisted commits churned less than human lines ({a} vs {h}).")
    }
}

pub fn text(r: &Report, color: bool) -> String {
    let p = Pal(color);
    let mut o = String::new();
    let _ = writeln!(o, "{} {} {}", p.bold(&p.rgb(ACCENT, "aichurn")), p.dim("·"), p.bold(&r.repo));
    if r.commits == 0 {
        let _ = writeln!(o, "\nNo commits to replay in this range.");
        return o;
    }
    let _ = writeln!(o, "{}", p.dim(&format!("replayed {} commits · {} → {} · churn window {} days", thousands(r.commits), r.from, r.to, r.window_days)));
    let _ = writeln!(o);
    let t = &r.total.all;
    let big = |label: &str, v: String, what: &str| format!("  {}  {}  {}", pad(label, 10, false), p.bold(&pad(&v, 6, true)), p.dim(what));
    let _ =
        writeln!(o, "{}", big("churn", side_or_dash(t, Side::churn_rate), &format!("of new lines were rewritten or deleted within {} days", r.window_days)));
    let _ = writeln!(o, "{}", big("copy/paste", side_or_dash(t, Side::dup_rate), "of new lines repeat a block of 6+ lines that already existed"));
    let _ = writeln!(o, "{}", big("moved", side_or_dash(t, Side::moved_rate), "of added lines were moved code (refactoring)"));
    let _ = writeln!(o);

    // AI vs human
    if r.total.ai.new_lines > 0 {
        let share = r.ai_share().map(pct).unwrap_or_default();
        let agents: Vec<String> = r.agents.iter().take(3).map(|(n, c)| format!("{n} {c}")).collect();
        let _ = writeln!(o, "  {} {}", p.bold("AI-assisted vs human"), p.dim(&format!("· {share} of new lines · commits by {}", agents.join(", "))));
        let _ = writeln!(
            o,
            "{}",
            p.dim(&format!("  {} {} {} {}", pad("", 14, false), pad("new lines", 10, true), pad("churn", 8, true), pad("copy/paste", 11, true)))
        );
        for (name, s, col) in [("AI-assisted", &r.total.ai, AI), ("Human", &r.total.human, (201, 209, 217))] {
            let _ = writeln!(
                o,
                "  {} {} {} {}",
                p.rgb(col, &pad(name, 14, false)),
                pad(&thousands(s.new_lines), 10, true),
                pad(&side_or_dash(s, Side::churn_rate), 8, true),
                pad(&side_or_dash(s, Side::dup_rate), 11, true)
            );
        }
        match r.ai_ratio() {
            Some(x) => {
                let mark = if x >= 1.1 {
                    p.rgb(BAD, "▲")
                } else if x <= 0.9 {
                    p.rgb(GOOD, "▼")
                } else {
                    p.dim("=")
                };
                let _ = writeln!(o, "  {mark} {}", ai_verdict(r, x));
            }
            None => {
                let _ = writeln!(
                    o,
                    "  {}",
                    p.dim(&format!("Too few AI-assisted lines with a full {}-day window to compare (need {MIN_AI_LINES}).", r.window_days))
                );
            }
        }
        let _ = writeln!(o);
    } else {
        let _ = writeln!(o, "  {}", p.dim("No AI-assisted commits found (agents that sign commits: Claude Code, Cursor, Copilot, Codex, Aider, …)."));
        let _ = writeln!(o);
    }

    // per period
    let unit = match r.grain {
        Grain::Quarter => "quarter",
        Grain::Month => "month",
    };
    let _ = writeln!(
        o,
        "{}",
        p.dim(&format!(
            "  {} {} {} {} {} {} {}",
            pad(unit, 9, false),
            pad("commits", 8, true),
            pad("AI", 6, true),
            pad("new lines", 10, true),
            pad("churn", 7, true),
            pad("copy/paste", 11, true),
            pad("moved", 7, true)
        ))
    );
    for (k, per) in &r.periods {
        let s = &per.all;
        let ai_c = if s.commits > 0 { pct(per.ai.commits as f64 / s.commits as f64) } else { "–".into() };
        let ai_c = if per.ai.commits > 0 { p.rgb(AI, &pad(&ai_c, 6, true)) } else { p.dim(&pad(&ai_c, 6, true)) };
        let mut churn = if s.eligible >= 50 { side_or_dash(s, Side::churn_rate) } else { "–".into() };
        if r.is_partial(k) && churn != "–" {
            churn.push('*');
        }
        let _ = writeln!(
            o,
            "  {} {} {} {} {} {} {}",
            pad(k, 9, false),
            pad(&thousands(s.commits), 8, true),
            ai_c,
            pad(&thousands(s.new_lines), 10, true),
            pad(&churn, 7, true),
            pad(&side_or_dash(s, Side::dup_rate), 11, true),
            pad(&side_or_dash(s, Side::moved_rate), 7, true)
        );
    }
    let series = r.churn_series();
    if series.len() > 1 {
        let vals: Vec<f64> = series.iter().map(|x| x.1).collect();
        let _ = writeln!(
            o,
            "\n  {}  {}  {}",
            p.dim("churn trend"),
            p.rgb(ACCENT, &sparkline(&vals)),
            p.dim(&format!("({} {} → {} {})", series[0].0, pct(vals[0]), series[series.len() - 1].0, pct(vals[vals.len() - 1])))
        );
    }
    if r.periods.iter().any(|(k, p)| r.is_partial(k) && p.all.eligible >= 50) {
        let _ = writeln!(o, "  {}", p.dim(&format!("* period ended less than {} days ago: churn will still rise", r.window_days)));
    }
    if r.skipped_bulk + r.skipped_bots > 0 {
        let _ = writeln!(
            o,
            "  {}",
            p.dim(&format!(
                "not counted (replayed only): {} bot commits, {} bulk commits over the size limit",
                thousands(r.skipped_bots),
                thousands(r.skipped_bulk)
            ))
        );
    }
    o
}

pub fn markdown(r: &Report) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "### aichurn: {}\n\n{}\n", r.repo, headline(r));
    if r.commits == 0 {
        return o;
    }
    let _ = writeln!(o, "{} commits replayed, {} to {}, churn window {} days.\n", thousands(r.commits), r.from, r.to, r.window_days);
    let _ = writeln!(o, "| | New lines | Churn | Copy/paste | Moved |\n|---|---:|---:|---:|---:|");
    for (n, s) in [("**All**", &r.total.all), ("AI-assisted", &r.total.ai), ("Human", &r.total.human)] {
        let _ = writeln!(
            o,
            "| {n} | {} | {} | {} | {} |",
            thousands(s.new_lines),
            side_or_dash(s, Side::churn_rate),
            side_or_dash(s, Side::dup_rate),
            side_or_dash(s, Side::moved_rate)
        );
    }
    let _ = writeln!(o, "\n| Period | Commits | AI-assisted commits | New lines | Churn | Copy/paste | Moved |\n|---|---:|---:|---:|---:|---:|---:|");
    for (k, per) in &r.periods {
        let s = &per.all;
        let ai_c = if s.commits > 0 { pct(per.ai.commits as f64 / s.commits as f64) } else { "–".into() };
        let mut churn = if s.eligible >= 50 { side_or_dash(s, Side::churn_rate) } else { "–".into() };
        if r.is_partial(k) && churn != "–" {
            churn += " (partial)";
        }
        let _ = writeln!(
            o,
            "| {k} | {} | {ai_c} | {} | {churn} | {} | {} |",
            thousands(s.commits),
            thousands(s.new_lines),
            side_or_dash(s, Side::dup_rate),
            side_or_dash(s, Side::moved_rate)
        );
    }
    let _ = writeln!(o, "\n<sub>Generated by [aichurn](https://github.com/celaldrg/aichurn).</sub>");
    o
}

fn jstr(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn jnum(x: Option<f64>) -> String {
    x.map(|v| format!("{:.6}", v).trim_end_matches('0').trim_end_matches('.').to_string()).unwrap_or_else(|| "null".into())
}

fn jside(s: &Side) -> String {
    format!(
        "{{\"commits\":{},\"new_lines\":{},\"eligible_lines\":{},\"churned_lines\":{},\"duplicated_lines\":{},\"moved_lines\":{},\"churn_rate\":{},\"copy_paste_rate\":{},\"moved_rate\":{}}}",
        s.commits,
        s.new_lines,
        s.eligible,
        s.churned,
        s.duplicated,
        s.moved,
        jnum(s.churn_rate()),
        jnum(s.dup_rate()),
        jnum(s.moved_rate())
    )
}

pub fn json(r: &Report) -> String {
    let mut o = String::new();
    let _ = write!(
        o,
        "{{\"repo\":{},\"from\":{},\"to\":{},\"window_days\":{},\"grain\":{},\"commits\":{},\"bulk_commits_skipped\":{},\"bot_commits_skipped\":{},\"headline\":{},\"ai_share_of_new_lines\":{},\"ai_to_human_churn_ratio\":{},",
        jstr(&r.repo),
        jstr(&r.from),
        jstr(&r.to),
        r.window_days,
        jstr(match r.grain {
            Grain::Quarter => "quarter",
            Grain::Month => "month",
        }),
        r.commits,
        r.skipped_bulk,
        r.skipped_bots,
        jstr(&headline(r)),
        jnum(r.ai_share()),
        jnum(r.ai_ratio())
    );
    let agents: Vec<String> = r.agents.iter().map(|(n, c)| format!("{{\"name\":{},\"commits\":{c}}}", jstr(n))).collect();
    let _ = write!(o, "\"agents\":[{}],", agents.join(","));
    let _ = write!(o, "\"total\":{{\"all\":{},\"ai\":{},\"human\":{}}},", jside(&r.total.all), jside(&r.total.ai), jside(&r.total.human));
    let per: Vec<String> = r
        .periods
        .iter()
        .map(|(k, p)| {
            format!(
                "{{\"period\":{},\"partial\":{},\"all\":{},\"ai\":{},\"human\":{}}}",
                jstr(k),
                r.is_partial(k),
                jside(&p.all),
                jside(&p.ai),
                jside(&p.human)
            )
        })
        .collect();
    let _ = write!(o, "\"periods\":[{}]}}", per.join(","));
    o.push('\n');
    o
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn text_width(s: &str) -> f64 {
    s.chars()
        .map(|c| match c {
            'i' | 'l' | '.' | ',' | ':' | ';' | '|' | '!' | '\'' | ' ' | '·' => 3.6,
            'm' | 'w' | 'M' | 'W' | '%' | '×' => 10.0,
            'A'..='Z' => 7.6,
            _ => 6.6,
        })
        .sum()
}

pub fn badge_value(r: &Report) -> String {
    let mut v = side_or_dash(&r.total.all, Side::churn_rate);
    if let Some(x) = r.ai_ratio() {
        v += &format!(" · AI {x:.1}×");
    }
    v
}

pub fn badge(label: &str, value: &str) -> String {
    let lw = (text_width(label) + 12.0).round();
    let vw = (text_width(value) + 12.0).round();
    let w = lw + vw;
    let t = esc(&format!("{label}: {value}"));
    let (l, v) = (esc(label), esc(value));
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="20" role="img" aria-label="{t}">
<title>{t}</title>
<linearGradient id="s" x2="0" y2="100%"><stop offset="0" stop-color="#bbb" stop-opacity=".1"/><stop offset="1" stop-opacity=".1"/></linearGradient>
<clipPath id="r"><rect width="{w}" height="20" rx="3" fill="#fff"/></clipPath>
<g clip-path="url(#r)"><rect width="{lw}" height="20" fill="#555"/><rect x="{lw}" width="{vw}" height="20" fill="#0d9488"/><rect width="{w}" height="20" fill="url(#s)"/></g>
<g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">
<text x="{lx}" y="15" fill="#010101" fill-opacity=".3">{l}</text><text x="{lx}" y="14">{l}</text>
<text x="{vx}" y="15" fill="#010101" fill-opacity=".3">{v}</text><text x="{vx}" y="14">{v}</text>
</g></svg>
"##,
        lx = lw / 2.0,
        vx = lw + vw / 2.0
    )
}

/// Shareable report card with a churn chart per period.
pub fn card(r: &Report) -> String {
    const W: f64 = 600.0;
    const H: f64 = 330.0;
    let font = "font-family=\"-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif\"";
    let mut o = String::new();
    let _ =
        writeln!(o, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="{}">"#, esc(&headline(r)));
    let _ = writeln!(o, "<rect width=\"{W}\" height=\"{H}\" rx=\"14\" fill=\"#0d1117\" stroke=\"#30363d\"/>\n<g {font}>");
    let _ = writeln!(
        o,
        r##"<text x="28" y="40" font-size="13" fill="#2dd4bf" font-weight="600">aichurn</text><text x="88" y="40" font-size="13" fill="#7d8590">{} · {} commits</text>"##,
        esc(&r.repo),
        thousands(r.commits)
    );
    let _ = writeln!(o, r##"<text x="28" y="98" font-size="52" font-weight="700" fill="#e6edf3">{}</text>"##, side_or_dash(&r.total.all, Side::churn_rate));
    let _ = writeln!(o, r##"<text x="28" y="124" font-size="13" fill="#7d8590">of new lines rewritten within {} days</text>"##, r.window_days);
    let stat = |o: &mut String, x: f64, y: f64, label: &str, v: String, fill: &str| {
        let _ = writeln!(
            o,
            r##"<text x="{x}" y="{y}" font-size="12" fill="#7d8590">{label}</text><text x="{x}" y="{}" font-size="22" font-weight="600" fill="{fill}">{v}</text>"##,
            y + 26.0
        );
    };
    if r.ai_ratio().is_some() {
        stat(&mut o, 340.0, 62.0, "churn · AI-assisted", side_or_dash(&r.total.ai, Side::churn_rate), "#fb923c");
        stat(&mut o, 470.0, 62.0, "churn · human", side_or_dash(&r.total.human, Side::churn_rate), "#e6edf3");
    } else {
        stat(&mut o, 340.0, 62.0, "AI share of new lines", r.ai_share().map(pct).unwrap_or_else(|| "0%".into()), "#fb923c");
    }
    stat(&mut o, 340.0, 112.0, "copy/paste", side_or_dash(&r.total.all, Side::dup_rate), "#e6edf3");
    stat(&mut o, 470.0, 112.0, "moved", side_or_dash(&r.total.all, Side::moved_rate), "#e6edf3");

    // bar chart of churn per period
    let series = r.churn_series();
    let (x0, y0, cw, ch) = (28.0, 290.0, W - 56.0, 110.0);
    let max = series.iter().map(|s| s.1).fold(0.0, f64::max).max(0.01);
    let n = series.len().max(1) as f64;
    let bw = (cw / n).min(60.0);
    let _ = writeln!(o, r##"<line x1="{x0}" y1="{y0}" x2="{}" y2="{y0}" stroke="#30363d"/>"##, x0 + cw);
    let by_key: std::collections::HashMap<&str, &crate::engine::Period> = r.periods.iter().map(|(k, p)| (k.as_str(), p)).collect();
    for (i, (k, v)) in series.iter().enumerate() {
        let h = (v / max) * ch;
        let x = x0 + i as f64 * bw;
        let _ = write!(
            o,
            r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="2" fill="#14b8a6"><title>{k}: {}</title></rect>"##,
            x + 2.0,
            y0 - h,
            bw - 4.0,
            h.max(1.0),
            pct(*v)
        );
        // part of the bar that is churned AI-assisted lines
        if let Some(p) = by_key.get(k) {
            if p.all.churned > 0 && p.ai.churned > 0 {
                let ah = h * p.ai.churned as f64 / p.all.churned as f64;
                let _ = write!(o, r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="#fb923c"/>"##, x + 2.0, y0 - ah, bw - 4.0, ah);
            }
        }
        if series.len() <= 12 || i % 2 == 0 {
            let _ = write!(o, r##"<text x="{:.1}" y="{}" font-size="9" fill="#7d8590" text-anchor="middle">{}</text>"##, x + bw / 2.0, y0 + 13.0, esc(k));
        }
        o.push('\n');
    }
    if r.total.ai.churned > 0 {
        let _ = writeln!(
            o,
            r##"<rect x="{}" y="{}" width="9" height="9" fill="#fb923c"/><text x="{}" y="{}" font-size="11" fill="#7d8590">from AI-assisted commits</text>"##,
            W - 180.0,
            y0 - ch - 16.0,
            W - 166.0,
            y0 - ch - 8.0
        );
    }
    let _ = writeln!(
        o,
        r##"<text x="28" y="{}" font-size="11" fill="#7d8590">churn per {} · {} → {} · github.com/celaldrg/aichurn</text>"##,
        y0 - ch - 8.0,
        if r.grain == Grain::Quarter { "quarter" } else { "month" },
        r.from,
        r.to
    );
    o.push_str("</g></svg>\n");
    o
}
