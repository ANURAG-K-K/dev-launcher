//! Sequential launcher / dependency orderer (F7/F8, architecture §3.5; detailed design §3).
//!
//! Builds a dependency graph from `repository_dependencies`, computes a valid launch order via
//! topological sort (Kahn's algorithm), rejects cycles before starting anything, then drives the
//! process manager one repo at a time with the configured `launch_delay_ms` between starts.
//! v1 approximates readiness with the fixed delay (health checks are deferred, R1).

use std::collections::{HashMap, HashSet};

/// Compute a launch order over `nodes` that respects dependency `edges`.
/// Each edge is `(dependent, depends_on)`: `dependent` must be launched AFTER `depends_on`.
/// The returned order lists every node exactly once, with each node appearing after all nodes it
/// depends on. Ties (nodes with no remaining dependency between them) are broken by the order the
/// nodes appear in `nodes` (stable).
///
/// Edges whose endpoints are not BOTH present in `nodes` are ignored (a dependency on a node
/// outside the set does not constrain ordering here - the caller handles "blocked" repos).
///
/// Returns `Err(cyclic)` with the node ids that remain in a cycle (non-zero in-degree after Kahn's
/// algorithm drains) if the graph is not a DAG.
pub fn topological_order(nodes: &[i64], edges: &[(i64, i64)]) -> Result<Vec<i64>, Vec<i64>> {
    let node_set: HashSet<i64> = nodes.iter().copied().collect();

    // Build in-degree + adjacency (depends_on -> dependents) only over edges fully inside `nodes`,
    // skipping self-edges and duplicates. Iterate `edges` in input order (not via a HashSet) so
    // adjacency lists stay deterministic.
    let mut in_degree: HashMap<i64, usize> = nodes.iter().map(|&n| (n, 0)).collect();
    let mut dependents: HashMap<i64, Vec<i64>> = nodes.iter().map(|&n| (n, Vec::new())).collect();
    let mut seen_edges: HashSet<(i64, i64)> = HashSet::new();

    for &(dependent, depends_on) in edges {
        if dependent == depends_on {
            continue; // self-edge: unsatisfiable, treated as no constraint, not a cycle
        }
        if !node_set.contains(&dependent) || !node_set.contains(&depends_on) {
            continue; // dependency outside the launch set - ignore
        }
        if !seen_edges.insert((dependent, depends_on)) {
            continue; // duplicate edge - don't inflate in-degree
        }
        *in_degree.get_mut(&dependent).unwrap() += 1;
        dependents.get_mut(&depends_on).unwrap().push(dependent);
    }

    // Kahn's algorithm, stable: repeatedly scan `nodes` in original order for newly-zero
    // in-degree nodes rather than using an arbitrary queue, so ties keep `nodes` order.
    let mut emitted: HashSet<i64> = HashSet::new();
    let mut order: Vec<i64> = Vec::with_capacity(nodes.len());

    loop {
        let mut progressed = false;
        for &n in nodes {
            if emitted.contains(&n) {
                continue;
            }
            if in_degree[&n] == 0 {
                emitted.insert(n);
                order.push(n);
                progressed = true;
                for &dependent in &dependents[&n] {
                    *in_degree.get_mut(&dependent).unwrap() -= 1;
                }
            }
        }
        if !progressed {
            break;
        }
    }

    if order.len() == nodes.len() {
        Ok(order)
    } else {
        let cyclic: Vec<i64> = nodes.iter().copied().filter(|n| !emitted.contains(n)).collect();
        Err(cyclic)
    }
}

#[cfg(test)]
mod tests {
    use super::topological_order;

    #[test]
    fn no_edges_preserves_input_order() {
        let nodes = [3, 1, 2];
        let result = topological_order(&nodes, &[]).unwrap();
        assert_eq!(result, vec![3, 1, 2]);
    }

    #[test]
    fn simple_chain_orders_dependencies_first() {
        // 1 depends on 2, 2 depends on 3 -> 3, 2, 1
        let nodes = [3, 1, 2];
        let edges = [(1, 2), (2, 3)];
        let result = topological_order(&nodes, &edges).unwrap();
        assert_eq!(result, vec![3, 2, 1]);
    }

    #[test]
    fn diamond_resolves_to_valid_order() {
        // a depends on b and c; b and c both depend on d.
        // edges are (dependent, depends_on).
        let nodes = [1, 2, 3, 4]; // a=1, b=2, c=3, d=4
        let edges = [(1, 2), (1, 3), (2, 4), (3, 4)];
        let result = topological_order(&nodes, &edges).unwrap();

        assert_eq!(result.len(), 4);
        let pos = |id: i64| result.iter().position(|&x| x == id).unwrap();
        assert!(pos(4) < pos(2)); // d before b
        assert!(pos(4) < pos(3)); // d before c
        assert!(pos(2) < pos(1)); // b before a
        assert!(pos(3) < pos(1)); // c before a
        assert_eq!(pos(4), 0); // d has no deps, first ready
        assert_eq!(pos(1), 3); // a depends on both, last
    }

    #[test]
    fn independent_nodes_keep_stable_order() {
        let nodes = [5, 2, 8, 1];
        let result = topological_order(&nodes, &[]).unwrap();
        assert_eq!(result, vec![5, 2, 8, 1]);
    }

    #[test]
    fn direct_cycle_is_reported() {
        let nodes = [1, 2];
        let edges = [(1, 2), (2, 1)];
        let err = topological_order(&nodes, &edges).unwrap_err();
        assert_eq!(err.len(), 2);
        assert!(err.contains(&1));
        assert!(err.contains(&2));
    }

    #[test]
    fn transitive_cycle_is_reported() {
        // A depends on B, B depends on C, C depends on A.
        let nodes = [10, 20, 30];
        let edges = [(10, 20), (20, 30), (30, 10)];
        let err = topological_order(&nodes, &edges).unwrap_err();
        assert_eq!(err.len(), 3);
        assert!(err.contains(&10));
        assert!(err.contains(&20));
        assert!(err.contains(&30));
    }

    #[test]
    fn edge_to_unknown_node_is_ignored() {
        let nodes = [1, 2];
        // 1 depends on 99, which isn't in `nodes` - should not panic or constrain ordering.
        let edges = [(1, 99)];
        let result = topological_order(&nodes, &edges).unwrap();
        assert_eq!(result, vec![1, 2]);
    }

    #[test]
    fn duplicate_edges_do_not_cause_false_cycle() {
        let nodes = [1, 2];
        let edges = [(1, 2), (1, 2), (1, 2)];
        let result = topological_order(&nodes, &edges).unwrap();
        assert_eq!(result, vec![2, 1]);
    }

    #[test]
    fn self_edge_is_ignored_not_a_cycle() {
        let nodes = [1, 2];
        let edges = [(1, 1), (1, 2)];
        let result = topological_order(&nodes, &edges).unwrap();
        assert_eq!(result, vec![2, 1]);
    }
}
