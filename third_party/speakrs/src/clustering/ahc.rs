use kodama::{Method, linkage};
use ndarray::{Array2, ArrayView2};

use crate::utils::l2_normalize_rows;

mod flat;
use flat::flat_clusters;

/// Agglomerative hierarchical clustering settings for speaker embeddings.
#[derive(Debug, Clone, Copy)]
pub struct AhcConfig {
    /// Maximum dendrogram merge distance to keep observations in one flat cluster.
    pub threshold: f32,
}

impl Default for AhcConfig {
    fn default() -> Self {
        Self { threshold: 0.6 }
    }
}

pub fn cluster(embeddings: &ArrayView2<f32>, config: AhcConfig) -> Vec<usize> {
    let observations = embeddings.nrows();
    if observations == 0 {
        return Vec::new();
    }
    if observations == 1 {
        return vec![0];
    }

    let normalized = l2_normalize_rows(embeddings);
    let mut condensed = condensed_euclidean(&normalized);
    let dendrogram = linkage(&mut condensed, observations, Method::Centroid);
    flat_clusters(observations, dendrogram.steps(), config.threshold)
}

fn condensed_euclidean(embeddings: &Array2<f32>) -> Vec<f32> {
    let observations = embeddings.nrows();
    let mut condensed = Vec::with_capacity(observations * (observations - 1) / 2);
    for row in 0..observations.saturating_sub(1) {
        for col in row + 1..observations {
            let lhs = embeddings.row(row);
            let rhs = embeddings.row(col);
            let distance = lhs
                .iter()
                .zip(rhs.iter())
                .map(|(left, right)| {
                    let delta = left - right;
                    delta * delta
                })
                .sum::<f32>()
                .sqrt();
            condensed.push(distance);
        }
    }
    condensed
}

#[cfg(test)]
mod tests {
    use kodama::Step;
    use ndarray::{Array1, Array2, array};
    use ndarray_npy::ReadNpyExt;
    use std::fs::File;
    use std::path::PathBuf;

    use super::*;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn separates_two_clusters() {
        let embeddings = array![[1.0, 0.0], [0.95, 0.05], [-1.0, 0.0], [-0.95, -0.05],];

        let labels = cluster(&embeddings.view(), AhcConfig { threshold: 0.6 });

        assert_eq!(labels[0], labels[1]);
        assert_eq!(labels[2], labels[3]);
        assert_ne!(labels[0], labels[2]);
    }

    #[test]
    fn flat_clusters_follow_scipy_leader_order() {
        let steps = vec![
            Step::new(2, 3, 0.1, 2),
            Step::new(0, 1, 1.0, 2),
            Step::new(4, 5, 10.45, 4),
        ];

        let labels = flat_clusters(4, &steps, 1.0);

        assert_eq!(labels, vec![1, 1, 0, 0]);
    }

    #[test]
    fn cluster_matches_scipy_label_order_on_toy_example() {
        let embeddings = array![[1.0, 0.0], [0.9, 0.3], [0.0, 1.0], [0.05, 1.0],];

        let labels = cluster(&embeddings.view(), AhcConfig { threshold: 0.6 });

        assert_eq!(labels, vec![1, 1, 0, 0]);
    }

    #[test]
    fn cluster_matches_python_fixture() {
        let embeddings: Array2<f32> =
            Array2::read_npy(File::open(fixture_path("pipeline_train_embeddings.npy")).unwrap())
                .unwrap();
        let expected: Array1<i64> =
            Array1::read_npy(File::open(fixture_path("pipeline_ahc_clusters.npy")).unwrap())
                .unwrap();

        let labels = cluster(&embeddings.view(), AhcConfig::default());

        assert_eq!(labels.len(), expected.len());
        for (lhs, rhs) in labels.iter().zip(expected.iter()) {
            assert_eq!(*lhs as i64, *rhs);
        }
    }
}
