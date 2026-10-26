// Exercise the production vendor implementation without exposing a public backend API.
#[path = "../../../third_party/speakrs/src/clustering/ahc/flat.rs"]
mod flat;

use kodama::Step;

#[test]
fn keeps_left_first_cluster_labels() {
    let steps = [
        Step::new(2, 3, 0.1, 2),
        Step::new(0, 1, 1.0, 2),
        Step::new(4, 5, 10.45, 4),
    ];
    assert_eq!(flat::flat_clusters(4, &steps, 1.0), [1, 1, 0, 0]);
    assert!(flat::flat_clusters(0, &[], 0.6).is_empty());
    assert_eq!(flat::flat_clusters(1, &[], 0.6), [0]);
}

#[test]
fn keeps_mixed_subtrees_and_inclusive_threshold() {
    let steps = [
        Step::new(0, 1, 0.2, 2),
        Step::new(2, 3, 0.8, 2),
        Step::new(4, 5, 0.3, 2),
        Step::new(6, 7, 1.2, 4),
        Step::new(8, 9, 2.0, 6),
    ];
    assert_eq!(flat::flat_clusters(6, &steps, 0.6), [1, 1, 2, 3, 0, 0]);
    assert_eq!(flat::flat_clusters(6, &steps, 0.8), [1, 1, 2, 2, 0, 0]);
    assert_eq!(flat::flat_clusters(6, &steps, 2.0), [0; 6]);
}

#[test]
fn rejects_a_low_parent_when_a_descendant_exceeds_threshold() {
    // SciPy fcluster(distance=0.6): all three observations stay separate.
    let steps = [Step::new(0, 1, 0.8, 2), Step::new(2, 3, 0.5, 3)];
    assert_eq!(flat::flat_clusters(3, &steps, 0.6), [1, 2, 0]);
    assert_eq!(flat::flat_clusters(3, &steps, 0.8), [0; 3]);
}

#[test]
fn preserves_valid_sibling_clusters_below_an_inversion() {
    let steps = [
        Step::new(0, 1, 0.8, 2),
        Step::new(2, 3, 0.1, 2),
        Step::new(4, 5, 0.5, 3),
        Step::new(6, 7, 0.55, 5),
    ];
    assert_eq!(flat::flat_clusters(5, &steps, 0.6), [2, 3, 0, 0, 1]);
}

#[test]
fn centroid_linkage_inversion_does_not_merge_over_threshold_pairs() {
    // Unit vectors 120 degrees apart have equal pair distances sqrt(3).
    // The final centroid merge is closer (1.5), although no pair is within 1.6.
    let mut distances = vec![3.0f32.sqrt(); 3];
    let tree = kodama::linkage(&mut distances, 3, kodama::Method::Centroid);
    assert!(tree.steps()[1].dissimilarity < tree.steps()[0].dissimilarity);
    let labels = flat::flat_clusters(3, tree.steps(), 1.6);
    assert_ne!(labels[0], labels[1]);
    assert_ne!(labels[0], labels[2]);
    assert_ne!(labels[1], labels[2]);
}

// A valid left-deep tree defeats right-tail-call elimination in optimized builds.
fn left_deep_tree(observations: usize, height: f32) -> Vec<Step<f32>> {
    let mut steps = Vec::with_capacity(observations - 1);
    steps.push(Step::new(0, 1, height, 2));
    let mut root = observations;
    for pair in 1..observations / 2 {
        let pair_node = observations + steps.len();
        steps.push(Step::new(pair * 2, pair * 2 + 1, height, 2));
        steps.push(Step::new(root, pair_node, height, (pair + 1) * 2));
        root = observations + steps.len() - 1;
    }
    steps
}

fn verify_deep_tree(mode: &str, test_name: &str) {
    const N: usize = 100_000;
    if std::env::var("MEETNOTE_AHC_DEEP_TREE").as_deref() == Ok(mode) {
        let collapsed = mode == "collapsed";
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                let steps = left_deep_tree(N, if collapsed { 0.1 } else { 2.0 });
                let labels = flat::flat_clusters(N, &steps, 0.6);
                assert_eq!(labels.len(), N);
                for (index, label) in labels.into_iter().enumerate() {
                    assert_eq!(label, if collapsed { 0 } else { index });
                }
            })
            .unwrap()
            .join()
            .unwrap();
        println!("AHC_DEEP_TREE_VERIFIED:{mode}:{N}");
        return;
    }
    // A stack overflow aborts the child, never the entire workspace test harness.
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env("MEETNOTE_AHC_DEEP_TREE", mode)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "deep {mode} cut failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout)
        .contains(&format!("AHC_DEEP_TREE_VERIFIED:{mode}:{N}")));
}

#[test]
fn cuts_deep_expanded_tree_without_stack_growth() {
    verify_deep_tree("expanded", "cuts_deep_expanded_tree_without_stack_growth");
}

#[test]
fn labels_deep_collapsed_tree_without_stack_growth() {
    verify_deep_tree(
        "collapsed",
        "labels_deep_collapsed_tree_without_stack_growth",
    );
}
