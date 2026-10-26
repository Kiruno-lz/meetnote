use kodama::Step;

pub(super) fn flat_clusters(
    observations: usize,
    steps: &[Step<f32>],
    threshold: f32,
) -> Vec<usize> {
    if observations == 0 {
        return Vec::new();
    }
    // Centroid linkage can invert merge heights. A distance cut must bound every
    // merge inside a subtree, not just its root (SciPy fcluster "distance").
    let mut maximum_heights: Vec<f32> = Vec::with_capacity(steps.len());
    for step in steps {
        let mut height = step.dissimilarity;
        for child in [step.cluster1, step.cluster2] {
            if child >= observations {
                height = height.max(maximum_heights[child - observations]);
            }
        }
        maximum_heights.push(height);
    }
    let mut labels = vec![usize::MAX; observations];
    let mut next_label = 0;
    // Keep traversal state on the heap: centroid linkage may produce a deep tree.
    let mut pending = vec![(observations + steps.len() - 1, None)];
    while let Some((node, inherited_label)) = pending.pop() {
        if node < observations {
            labels[node] = inherited_label.unwrap_or_else(|| {
                let label = next_label;
                next_label += 1;
                label
            });
            continue;
        }
        let step = &steps[node - observations];
        let label = inherited_label.or_else(|| {
            (maximum_heights[node - observations] <= threshold).then(|| {
                let label = next_label;
                next_label += 1;
                label
            })
        });
        // LIFO order preserves the existing left-first cluster numbering.
        pending.push((step.cluster2, label));
        pending.push((step.cluster1, label));
    }
    labels
}
