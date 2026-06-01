//! Commit-DAG lane assignment — the math behind gitk's colored graph column.
//!
//! Given commits in reverse-topological order (newest first) with their parent
//! SHAs, [`compute_graph`] assigns each commit a *lane* (column) and emits the
//! line segments to draw in the top and bottom halves of its row, so a branch
//! shows as a vertical line that forks at a parent and merges back at a child.
//!
//! The algorithm is the standard incremental one: walk rows top-to-bottom
//! keeping a set of *active lanes*, each tracking the SHA it's heading toward
//! (a parent it expects to reach). A commit takes the lane(s) that were waiting
//! for it (children merging in), then its first parent continues that lane
//! while extra parents (merges) open new lanes. It's pure and free of any
//! rendering, so it can be unit-tested directly.

const MAX_LANES: usize = 24;

/// Per-row graph geometry. Columns are lane indices; the renderer maps them to
/// x positions. Segments are `(column_at_edge, column_at_center)` for the top
/// half and `(column_at_center, column_at_edge)` for the bottom half.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GraphRow {
    /// Lane the commit's dot sits in.
    pub node_col: usize,
    /// Top-half segments: from a lane at the row's top edge down to the center.
    pub top: Vec<(usize, usize)>,
    /// Bottom-half segments: from the center down to a lane at the bottom edge.
    pub bottom: Vec<(usize, usize)>,
}

/// The full graph plus the number of lanes ever active (for gutter sizing).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Graph {
    pub rows: Vec<GraphRow>,
    pub lane_count: usize,
}

/// Compute the lane layout for `commits`, each `(sha, parent_shas)`, in
/// reverse-topological (newest-first) order.
pub fn compute_graph(commits: &[(String, Vec<String>)]) -> Graph {
    // Each active lane tracks the SHA it is heading toward (a parent).
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let mut lane_count = 1usize;

    for (id, parents) in commits {
        // Lanes whose expected commit is this one — children merging in.
        let incoming: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(i, l)| (l.as_deref() == Some(id.as_str())).then_some(i))
            .collect();

        let node_col = match incoming.first() {
            Some(&first) => first,
            None => free_slot(&mut lanes), // a tip: open a fresh lane
        };

        // Top half: every active lane draws a segment to the row center;
        // lanes waiting for this commit converge into the node's column.
        let mut top = Vec::new();
        for (i, lane) in lanes.iter().enumerate() {
            if lane.is_none() {
                continue;
            }
            if incoming.contains(&i) {
                top.push((i, node_col));
            } else {
                top.push((i, i));
            }
        }

        // The converging lanes are consumed by this commit.
        for &i in &incoming {
            lanes[i] = None;
        }
        if node_col < lanes.len() {
            lanes[node_col] = None;
        }

        // Route parents into lanes. The first parent continues the node's
        // lane; extra parents reuse a lane already heading to them, else open
        // a new one.
        let mut parent_cols = Vec::new();
        for (k, parent) in parents.iter().enumerate() {
            let col = if let Some(existing) = lanes
                .iter()
                .position(|l| l.as_deref() == Some(parent.as_str()))
            {
                existing
            } else if k == 0 {
                node_col
            } else {
                free_slot(&mut lanes)
            };
            ensure_len(&mut lanes, col);
            lanes[col] = Some(parent.clone());
            if !parent_cols.contains(&col) {
                parent_cols.push(col);
            }
        }

        // Bottom half: every still-active lane draws from the center to the
        // bottom edge; the node's parents start from the node's column.
        let mut bottom = Vec::new();
        for (j, lane) in lanes.iter().enumerate() {
            if lane.is_none() {
                continue;
            }
            if parent_cols.contains(&j) {
                bottom.push((node_col, j));
            } else {
                bottom.push((j, j));
            }
        }

        while matches!(lanes.last(), Some(None)) {
            lanes.pop();
        }

        for &(a, b) in top.iter().chain(bottom.iter()) {
            lane_count = lane_count.max(a + 1).max(b + 1);
        }
        lane_count = lane_count.max(node_col + 1).max(lanes.len());

        rows.push(GraphRow {
            node_col,
            top,
            bottom,
        });
    }

    Graph {
        rows,
        lane_count: lane_count.clamp(1, MAX_LANES),
    }
}

fn free_slot(lanes: &mut Vec<Option<String>>) -> usize {
    match lanes.iter().position(|l| l.is_none()) {
        Some(i) => i,
        None => {
            lanes.push(None);
            lanes.len() - 1
        }
    }
}

fn ensure_len(lanes: &mut Vec<Option<String>>, idx: usize) {
    if idx >= lanes.len() {
        lanes.resize(idx + 1, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, parents: &[&str]) -> (String, Vec<String>) {
        (
            id.to_string(),
            parents.iter().map(|p| p.to_string()).collect(),
        )
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let commits = [c("A", &["B"]), c("B", &["C"]), c("C", &[])];
        let g = compute_graph(&commits);
        assert_eq!(g.lane_count, 1);
        assert!(g.rows.iter().all(|r| r.node_col == 0));
        // The tip has nothing above it; the root has nothing below it.
        assert!(g.rows[0].top.is_empty());
        assert_eq!(g.rows[0].bottom, vec![(0, 0)]);
        assert!(g.rows[2].bottom.is_empty());
    }

    #[test]
    fn branch_then_merge_uses_two_lanes() {
        // A is a merge of B and D; both lead back to C.
        let commits = [
            c("A", &["B", "D"]),
            c("B", &["C"]),
            c("D", &["C"]),
            c("C", &["E"]),
            c("E", &[]),
        ];
        let g = compute_graph(&commits);
        assert_eq!(g.lane_count, 2, "branch should occupy a second lane");
        // The merge commit fans out to two lanes below it.
        assert_eq!(g.rows[0].node_col, 0);
        assert_eq!(g.rows[0].bottom, vec![(0, 0), (0, 1)]);
        // D (row 3) lives in lane 1 and merges back down into lane 0 at C.
        assert_eq!(g.rows[2].node_col, 1);
        assert_eq!(g.rows[2].bottom, vec![(1, 0)]);
        // Everything collapses back to one lane by the root.
        assert_eq!(g.rows[4].node_col, 0);
        assert!(g.rows[4].bottom.is_empty());
    }
}
