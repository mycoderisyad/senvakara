//! Run order. Actions follow their order connections (`after`,
//! `depends_on`); ties keep the order nodes were added. Actions on the same
//! machine run one at a time (one package manager, one sudo prompt), while
//! different machines may run side by side.

use crate::graph::{Binding, Workflow};
use crate::validate::{validate, CheckedNode, Problem};
use crate::Catalog;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Every action in a valid run order.
    pub steps: Vec<CheckedNode>,
    /// For each step (same index), the steps that must finish first.
    pub waits_for: Vec<Vec<usize>>,
}

/// Validates the workflow and orders its actions.
pub fn plan(wf: &Workflow, catalog: &Catalog) -> Result<Plan, Vec<Problem>> {
    let checked = validate(wf, catalog)?;
    let (order, preds) = sort(wf, &checked).map_err(|cycle| {
        vec![Problem {
            node: None,
            message: format!(
                "these nodes wait for each other in a loop: {}",
                cycle.join(", ")
            ),
        }]
    })?;
    let position: BTreeMap<usize, usize> =
        order.iter().enumerate().map(|(pos, &i)| (i, pos)).collect();
    let waits_for = order
        .iter()
        .map(|i| {
            let mut w: Vec<usize> = preds[*i].iter().map(|p| position[p]).collect();
            w.sort_unstable();
            w
        })
        .collect();
    let steps = order.into_iter().map(|i| checked[i].clone()).collect();
    Ok(Plan { steps, waits_for })
}

impl Plan {
    /// Steps grouped by the machine they run on, each lane in run order.
    pub fn lanes(&self) -> Vec<(Binding, Vec<usize>)> {
        let mut lanes: Vec<(Binding, Vec<usize>)> = Vec::new();
        for (i, step) in self.steps.iter().enumerate() {
            match lanes.iter_mut().find(|(b, _)| *b == step.target) {
                Some((_, list)) => list.push(i),
                None => lanes.push((step.target.clone(), vec![i])),
            }
        }
        lanes
    }

    /// Every step that waits, directly or not, for step `i`: what is skipped
    /// when `i` fails.
    pub fn downstream(&self, i: usize) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        let mut todo = vec![i];
        while let Some(cur) = todo.pop() {
            for (j, waits) in self.waits_for.iter().enumerate() {
                if waits.contains(&cur) && out.insert(j) {
                    todo.push(j);
                }
            }
        }
        out
    }

    pub fn needs_root(&self) -> bool {
        self.steps.iter().any(|s| s.needs_root)
    }
}

/// Topological order of `checked` (indexes) over the order connections.
/// On a loop, returns the ids of the nodes caught in it.
pub(crate) fn order(wf: &Workflow, checked: &[CheckedNode]) -> Result<Vec<usize>, Vec<String>> {
    sort(wf, checked).map(|(order, _)| order)
}

type Sorted = (Vec<usize>, Vec<BTreeSet<usize>>);

fn sort(wf: &Workflow, checked: &[CheckedNode]) -> Result<Sorted, Vec<String>> {
    let index: BTreeMap<&str, usize> = checked
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();
    let mut preds: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); checked.len()];
    for e in wf.edges.iter().filter(|e| e.kind.orders()) {
        if let (Some(&a), Some(&b)) = (index.get(e.from.as_str()), index.get(e.to.as_str())) {
            if a != b {
                preds[b].insert(a);
            }
        }
    }
    // Kahn's algorithm; the lowest index goes first so the order is stable.
    let mut waiting: Vec<usize> = preds.iter().map(BTreeSet::len).collect();
    let mut ready: BTreeSet<usize> = (0..checked.len()).filter(|&i| waiting[i] == 0).collect();
    let mut out = Vec::with_capacity(checked.len());
    while let Some(i) = ready.pop_first() {
        out.push(i);
        for (j, p) in preds.iter().enumerate() {
            if p.contains(&i) {
                waiting[j] -= 1;
                if waiting[j] == 0 {
                    ready.insert(j);
                }
            }
        }
    }
    if out.len() == checked.len() {
        Ok((out, preds))
    } else {
        Err((0..checked.len())
            .filter(|&i| waiting[i] > 0)
            .map(|i| checked[i].id.clone())
            .collect())
    }
}
