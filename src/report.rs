//! Summary numbers built from the engine's per-period counts.

use crate::engine::{Engine, Grain, Period, Side};

/// Minimum eligible AI-assisted lines before an AI vs human comparison is shown.
pub const MIN_AI_LINES: u64 = 300;

pub struct Report {
    pub repo: String,
    pub from: String,
    pub to: String,
    pub window_days: i64,
    pub grain: Grain,
    pub commits: u64,
    pub skipped_bulk: u64,
    pub skipped_bots: u64,
    pub agents: Vec<(&'static str, u64)>,
    pub periods: Vec<(String, Period)>,
    pub total: Period,
    pub head_time: i64,
}

impl Report {
    pub fn build(repo: &str, e: &Engine, window_days: i64, grain: Grain) -> Report {
        let periods = e.result();
        let mut total = Period::default();
        for (_, p) in &periods {
            for (dst, src) in [(&mut total.ai, &p.ai), (&mut total.human, &p.human)] {
                dst.new_lines += src.new_lines;
                dst.eligible += src.eligible;
                dst.churned += src.churned;
                dst.duplicated += src.duplicated;
                dst.moved += src.moved;
                dst.commits += src.commits;
            }
        }
        let total = total.merged();
        let mut agents: Vec<(&'static str, u64)> = e.agents.iter().map(|(k, v)| (*k, *v)).collect();
        agents.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        Report {
            repo: repo.to_string(),
            from: e.first_time.map(crate::engine::date).unwrap_or_default(),
            to: e.last_time.map(crate::engine::date).unwrap_or_default(),
            window_days,
            grain,
            commits: e.commits,
            skipped_bulk: e.skipped_bulk,
            skipped_bots: e.skipped_bots,
            agents,
            periods,
            total,
            head_time: e.head_time(),
        }
    }

    /// AI-assisted churn divided by human churn, when both are measurable.
    pub fn ai_ratio(&self) -> Option<f64> {
        if self.total.ai.eligible < MIN_AI_LINES {
            return None;
        }
        let a = self.total.ai.churn_rate()?;
        let h = self.total.human.churn_rate()?;
        (h > 0.0).then(|| a / h)
    }

    /// Share of new lines that came from AI-assisted commits.
    pub fn ai_share(&self) -> Option<f64> {
        let t = self.total.all.new_lines;
        (t > 0).then(|| self.total.ai.new_lines as f64 / t as f64)
    }

    /// True when some lines of the period are still inside their churn window.
    pub fn is_partial(&self, key: &str) -> bool {
        crate::engine::period_end(key).is_some_and(|end| end > self.head_time - self.window_days * crate::engine::DAY)
    }

    /// Churn per complete period with enough lines to be meaningful.
    pub fn churn_series(&self) -> Vec<(&str, f64)> {
        self.periods
            .iter()
            .filter(|(k, _)| !self.is_partial(k))
            .filter_map(|(k, p)| p.all.churn_rate().filter(|_| p.all.eligible >= 50).map(|r| (k.as_str(), r)))
            .collect()
    }
}

pub fn side_or_dash(s: &Side, f: fn(&Side) -> Option<f64>) -> String {
    f(s).map(pct).unwrap_or_else(|| "–".into())
}

pub fn pct(x: f64) -> String {
    let v = x * 100.0;
    if v > 0.0 && v < 0.1 {
        "<0.1%".into()
    } else if v >= 10.0 {
        format!("{v:.0}%")
    } else {
        format!("{v:.1}%")
    }
}

pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formatting() {
        assert_eq!(pct(0.0), "0.0%");
        assert_eq!(pct(0.0004), "<0.1%");
        assert_eq!(pct(0.079), "7.9%");
        assert_eq!(pct(0.5), "50%");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(12), "12");
    }
}
