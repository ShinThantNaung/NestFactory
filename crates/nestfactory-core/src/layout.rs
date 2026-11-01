//! Layered (Sugiyama-style) layout for the module graph: back edges removed by DFS,
//! longest-path layering, barycenter ordering, and wide layers wrapped into rows.

use std::collections::HashMap;

use crate::model::Graph;

const MAX_ROW: usize = 8;
const H_GAP: f64 = 72.0;
const V_GAP: f64 = 96.0;
const MARGIN: f64 = 64.0;

pub fn layered_layout(graph: &mut Graph, roots: &[String]) {
    let n = graph.nodes.len();
    if n == 0 {
        graph.width = 0.0;
        graph.height = 0.0;
        return;
    }
    let idx: HashMap<String, usize> = graph.nodes.iter().enumerate().map(|(i, nd)| (nd.id.clone(), i)).collect();
    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];
    for e in &graph.edges {
        if let (Some(&a), Some(&b)) = (idx.get(&e.from), idx.get(&e.to)) {
            if a != b && !adj[a].contains(&b) {
                adj[a].push(b);
            }
        }
    }

    // DFS order from roots, marking back edges.
    let mut order: Vec<usize> = roots.iter().filter_map(|r| idx.get(r).copied()).collect();
    for i in 0..n {
        if !order.contains(&i) {
            order.push(i);
        }
    }
    let mut state = vec![0u8; n];
    let mut discovery = vec![];
    let mut back = vec![vec![false; 0]; n];
    for (i, list) in adj.iter().enumerate() {
        back[i] = vec![false; list.len()];
    }
    for &start in &order {
        if state[start] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        state[start] = 1;
        discovery.push(start);
        while let Some(&mut (v, ref mut k)) = stack.last_mut() {
            if *k < adj[v].len() {
                let w = adj[v][*k];
                let ki = *k;
                *k += 1;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        discovery.push(w);
                        stack.push((w, 0));
                    }
                    1 => back[v][ki] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }

    // Longest-path layering on the DAG.
    let mut indeg = vec![0usize; n];
    for v in 0..n {
        for (k, &w) in adj[v].iter().enumerate() {
            if !back[v][k] {
                indeg[w] += 1;
            }
        }
    }
    let mut layer = vec![0usize; n];
    let mut queue: Vec<usize> = discovery.iter().copied().filter(|&v| indeg[v] == 0).collect();
    let mut qi = 0;
    while qi < queue.len() {
        let v = queue[qi];
        qi += 1;
        for (k, &w) in adj[v].iter().enumerate() {
            if back[v][k] {
                continue;
            }
            layer[w] = layer[w].max(layer[v] + 1);
            indeg[w] -= 1;
            if indeg[w] == 0 {
                queue.push(w);
            }
        }
    }

    let max_layer = layer.iter().copied().max().unwrap_or(0);
    let rank: HashMap<usize, usize> = discovery.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let mut layers: Vec<Vec<usize>> = vec![vec![]; max_layer + 1];
    for &v in &discovery {
        layers[layer[v]].push(v);
    }
    for l in layers.iter_mut() {
        l.sort_by_key(|v| rank[v]);
    }

    // Barycenter ordering: a few downward sweeps using predecessor positions.
    let mut preds: Vec<Vec<usize>> = vec![vec![]; n];
    for v in 0..n {
        for (k, &w) in adj[v].iter().enumerate() {
            if !back[v][k] {
                preds[w].push(v);
            }
        }
    }
    let mut pos = vec![0.0f64; n];
    for _ in 0..3 {
        for l in layers.iter_mut() {
            for (i, &v) in l.iter().enumerate() {
                pos[v] = i as f64;
            }
        }
        for li in 1..layers.len() {
            let mut keyed: Vec<(f64, usize)> = layers[li]
                .iter()
                .map(|&v| {
                    let p = &preds[v];
                    let key = if p.is_empty() { pos[v] } else { p.iter().map(|&u| pos[u]).sum::<f64>() / p.len() as f64 };
                    (key, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            layers[li] = keyed.into_iter().map(|(_, v)| v).collect();
            for (i, &v) in layers[li].iter().enumerate() {
                pos[v] = i as f64 * (layers[li - 1].len().max(1) as f64 / layers[li].len().max(1) as f64);
            }
        }
    }

    // Wrap wide layers into rows, then assign coordinates.
    let rows: Vec<Vec<usize>> = layers.into_iter().flat_map(|l| l.chunks(MAX_ROW).map(|c| c.to_vec()).collect::<Vec<_>>()).collect();
    let w = graph.nodes.iter().map(|n| n.w).fold(0.0, f64::max);
    let h = graph.nodes.iter().map(|n| n.h).fold(0.0, f64::max);
    let row_width = |k: usize| k as f64 * w + (k.saturating_sub(1)) as f64 * H_GAP;
    let max_width = rows.iter().map(|r| row_width(r.len())).fold(0.0, f64::max);
    for (ri, row) in rows.iter().enumerate() {
        let offset = MARGIN + (max_width - row_width(row.len())) / 2.0;
        for (j, &v) in row.iter().enumerate() {
            graph.nodes[v].x = offset + j as f64 * (w + H_GAP);
            graph.nodes[v].y = MARGIN + ri as f64 * (h + V_GAP);
        }
    }
    graph.width = max_width + 2.0 * MARGIN;
    graph.height = rows.len() as f64 * (h + V_GAP) - V_GAP + 2.0 * MARGIN;
}
