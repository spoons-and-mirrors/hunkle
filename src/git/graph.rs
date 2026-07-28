use std::collections::HashMap;

use super::{Commit, GraphCell};

pub(super) const GRAPH_COMMIT_LIMIT: usize = 5_000;
const MAX_GRAPH_LANES: usize = 64;

pub(super) struct PreparedGraph {
    pub(super) commits: Vec<Commit>,
    pub(super) width: usize,
    pub(super) truncated: bool,
}

pub(super) fn prepare(commits: Vec<Commit>) -> PreparedGraph {
    let (mut commits, commit_truncated) = cap(commits);
    let lane_truncated = layout(&mut commits);
    let width = commits
        .iter()
        .map(|commit| commit.graph.len())
        .max()
        .unwrap_or(1);
    PreparedGraph {
        commits,
        width,
        truncated: commit_truncated || lane_truncated,
    }
}

pub(super) fn cap(mut commits: Vec<Commit>) -> (Vec<Commit>, bool) {
    let truncated = commits.len() > GRAPH_COMMIT_LIMIT;
    commits.truncate(GRAPH_COMMIT_LIMIT);
    (commits, truncated)
}

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

pub(super) fn layout(commits: &mut [Commit]) -> bool {
    let mut oid_ids = HashMap::new();
    let mut next_oid = 0usize;
    for commit in commits.iter() {
        for oid in std::iter::once(&commit.oid).chain(commit.parents.iter()) {
            oid_ids.entry(oid.clone()).or_insert_with(|| {
                let id = next_oid;
                next_oid += 1;
                id
            });
        }
    }

    let mut lanes: Vec<Option<usize>> = Vec::new();
    let mut colors: Vec<usize> = Vec::new();
    let mut next_color = 0;
    let mut truncated = false;

    for commit in commits {
        let commit_id = oid_ids[&commit.oid];
        let incoming: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(index, oid)| (*oid == Some(commit_id)).then_some(index))
            .collect();

        let node = incoming.first().copied().unwrap_or_else(|| {
            if let Some(index) = lanes.iter().position(Option::is_none) {
                lanes[index] = Some(commit_id);
                colors[index] = next_color;
                next_color += 1;
                index
            } else {
                if lanes.len() < MAX_GRAPH_LANES {
                    lanes.push(Some(commit_id));
                    colors.push(next_color);
                    next_color += 1;
                    lanes.len() - 1
                } else {
                    truncated = true;
                    let index = MAX_GRAPH_LANES - 1;
                    lanes[index] = Some(commit_id);
                    colors[index] = next_color;
                    next_color += 1;
                    index
                }
            }
        });

        let before_len = lanes.len();
        let mut after = lanes.clone();
        for lane in incoming.iter().copied().skip(1) {
            after[lane] = None;
        }

        if let Some(first_parent) = commit.parents.first() {
            after[node] = Some(oid_ids[first_parent]);
        } else {
            after[node] = None;
        }

        let mut outgoing = Vec::new();
        for parent in commit.parents.iter().skip(1) {
            let parent_id = oid_ids[parent];
            let destination = after
                .iter()
                .position(|oid| *oid == Some(parent_id))
                .unwrap_or_else(|| {
                    if let Some(index) = after.iter().position(Option::is_none) {
                        after[index] = Some(parent_id);
                        colors[index] = next_color;
                        next_color += 1;
                        index
                    } else {
                        if after.len() < MAX_GRAPH_LANES {
                            after.push(Some(parent_id));
                            colors.push(next_color);
                            next_color += 1;
                            after.len() - 1
                        } else {
                            truncated = true;
                            let index = MAX_GRAPH_LANES - 1;
                            after[index] = Some(parent_id);
                            colors[index] = next_color;
                            next_color += 1;
                            index
                        }
                    }
                });
            outgoing.push(destination);
        }

        let lane_count = before_len.max(after.len()).max(node + 1);
        let mut masks = vec![0_u8; lane_count.saturating_mul(2).saturating_sub(1)];
        let mut cell_colors = vec![colors.get(node).copied().unwrap_or(0); masks.len()];

        for (index, lane) in lanes.iter().enumerate() {
            if lane.is_some() {
                masks[index * 2] |= UP;
                cell_colors[index * 2] = colors[index];
            }
        }
        for (index, lane) in after.iter().enumerate() {
            if lane.is_some() {
                masks[index * 2] |= DOWN;
                cell_colors[index * 2] = colors[index];
            }
        }

        for destination in incoming.iter().copied().skip(1).chain(outgoing) {
            connect(
                &mut masks,
                &mut cell_colors,
                node * 2,
                destination * 2,
                colors[node],
            );
        }

        commit.graph = masks
            .into_iter()
            .enumerate()
            .map(|(index, mask)| GraphCell {
                symbol: if index == node * 2 {
                    '●'
                } else {
                    glyph(mask)
                },
                color: cell_colors[index],
            })
            .collect();

        lanes = after;
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
            colors.pop();
        }
    }
    truncated
}

fn connect(masks: &mut [u8], colors: &mut [usize], from: usize, to: usize, color: usize) {
    let (left, right) = if from <= to { (from, to) } else { (to, from) };
    for index in left..=right {
        if index > left {
            masks[index] |= LEFT;
        }
        if index < right {
            masks[index] |= RIGHT;
        }
        colors[index] = color;
    }
}

fn glyph(mask: u8) -> char {
    match mask {
        0 => ' ',
        3 => '│',
        12 => '─',
        10 => '╭',
        6 => '╮',
        9 => '╰',
        5 => '╯',
        11 => '├',
        7 => '┤',
        14 => '┬',
        13 => '┴',
        15 => '┼',
        UP => '╵',
        DOWN => '╷',
        LEFT => '╴',
        RIGHT => '╶',
        _ => '┼',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_merge_linear_and_distinct_histories() {
        let mut merge = vec![
            commit("merge", &["left", "right"]),
            commit("left", &["base"]),
            commit("right", &["base"]),
            commit("base", &[]),
        ];
        layout(&mut merge);
        assert_eq!(symbols(&merge), ["●─╮", "● │", "│ ●", "●─╯"]);

        let mut linear = vec![
            commit("three", &["two"]),
            commit("two", &["one"]),
            commit("one", &[]),
        ];
        layout(&mut linear);
        assert_eq!(symbols(&linear), ["●", "●", "●"]);

        let mut distinct = vec![
            commit("main", &["base"]),
            commit("side", &["base"]),
            commit("base", &[]),
        ];
        layout(&mut distinct);
        assert_eq!(symbols(&distinct), ["●", "│ ●", "●─╯"]);
    }

    #[test]
    fn caps_graph_commits_and_reports_truncation() {
        let commits = (0..=GRAPH_COMMIT_LIMIT)
            .map(|index| commit(&index.to_string(), &[]))
            .collect();
        let prepared = prepare(commits);
        assert_eq!(prepared.commits.len(), GRAPH_COMMIT_LIMIT);
        assert!(prepared.truncated);
        assert!(
            prepared
                .commits
                .iter()
                .all(|commit| commit.graph.len() == 1 && commit.graph[0].symbol == '●')
        );
    }

    #[test]
    fn caps_pathological_graph_lane_growth() {
        let parents = (0..MAX_GRAPH_LANES * 2)
            .map(|index| format!("parent-{index}"))
            .collect::<Vec<_>>();
        let mut commits = vec![Commit {
            oid: "merge".to_owned(),
            parents,
            refs: Vec::new(),
            author: String::new(),
            date: String::new(),
            subject: String::new(),
            message: String::new(),
            graph: Vec::new(),
        }];

        assert!(layout(&mut commits));
        assert!(commits[0].graph.len() < MAX_GRAPH_LANES * 2);
    }

    fn symbols(commits: &[Commit]) -> Vec<String> {
        commits
            .iter()
            .map(|commit| commit.graph.iter().map(|cell| cell.symbol).collect())
            .collect()
    }

    fn commit(oid: &str, parents: &[&str]) -> Commit {
        Commit {
            oid: oid.to_owned(),
            parents: parents.iter().map(|parent| (*parent).to_owned()).collect(),
            refs: Vec::new(),
            author: "A".to_owned(),
            date: "2026-01-01".to_owned(),
            subject: oid.to_owned(),
            message: oid.to_owned(),
            graph: Vec::new(),
        }
    }
}
