//! Attributing merged branches to their merge commit.
//!
//! aichurn replays the first-parent history, so a merge commit carries all the
//! lines of the branch it merged. Merge commit messages rarely carry agent
//! trailers, so a merge counts as AI-assisted when any commit it brought in
//! was signed by an agent, the same rule a squash merge follows (GitHub keeps
//! every co-author trailer in the squashed message).

use std::collections::{HashMap, HashSet};

use crate::patch::Commit;

/// `commits` is every commit in the range (any order) with parents filled in;
/// `mainline` is the first-parent history, oldest first. Returns, for each
/// mainline merge, the agent index that signed most of the commits it merged.
pub fn merge_agents(commits: &[(Commit, Option<usize>)], mainline: &[String]) -> HashMap<String, usize> {
    let by_hash: HashMap<&str, &(Commit, Option<usize>)> = commits.iter().map(|c| (c.0.hash.as_str(), c)).collect();
    let main: HashSet<&str> = mainline.iter().map(|s| s.as_str()).collect();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut out = HashMap::new();
    for m in mainline {
        let Some((mc, _)) = by_hash.get(m.as_str()) else { continue };
        if mc.parents.len() < 2 {
            continue;
        }
        let mut counts: HashMap<usize, u32> = HashMap::new();
        let mut stack: Vec<&str> = mc.parents[1..].iter().map(|s| s.as_str()).collect();
        while let Some(h) = stack.pop() {
            if main.contains(h) || !visited.insert(h) {
                continue;
            }
            let Some((c, agent)) = by_hash.get(h) else { continue };
            if let Some(a) = agent {
                *counts.entry(*a).or_default() += 1;
            }
            stack.extend(c.parents.iter().map(|s| s.as_str()));
        }
        if let Some((a, _)) = counts.into_iter().max_by_key(|(a, n)| (*n, std::cmp::Reverse(*a))) {
            out.insert(m.clone(), a);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(h: &str, parents: &[&str]) -> Commit {
        Commit { hash: h.into(), parents: parents.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn merges_inherit_agents_from_their_branch() {
        // main: a - m1 - b - m2 ; m1 merges x1-x2 (x2 by agent 0); m2 merges y1 (human)
        let commits = vec![
            (c("a", &[]), None),
            (c("x1", &["a"]), None),
            (c("x2", &["x1"]), Some(0)),
            (c("m1", &["a", "x2"]), None),
            (c("b", &["m1"]), None),
            (c("y1", &["m1"]), None),
            (c("m2", &["b", "y1"]), None),
        ];
        let mainline: Vec<String> = ["a", "m1", "b", "m2"].iter().map(|s| s.to_string()).collect();
        let m = merge_agents(&commits, &mainline);
        assert_eq!(m.get("m1"), Some(&0));
        assert_eq!(m.get("m2"), None, "y1 is human and m1's branch was already counted");
    }
}
