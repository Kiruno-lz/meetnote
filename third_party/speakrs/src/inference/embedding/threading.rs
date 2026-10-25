// Reserve roughly one third of estimated parallelism for the host on larger
// devices; keep one/two-thread devices usable without an absolute thread cap.
pub(crate) fn cpu_embedding_threads(available: usize) -> usize {
    let available = available.max(1);
    if available <= 2 {
        available
    } else {
        available - available.div_ceil(3)
    }
}

#[cfg(test)]
mod tests {
    use super::cpu_embedding_threads;

    #[test]
    fn device_budget_grows_without_an_absolute_four_thread_ceiling() {
        for (available, expected) in [
            (0, 1),
            (1, 1),
            (2, 2),
            (3, 2),
            (4, 2),
            (6, 4),
            (8, 5),
            (10, 6),
            (16, 10),
            (32, 21),
            (64, 42),
            (128, 85),
        ] {
            assert_eq!(
                cpu_embedding_threads(available),
                expected,
                "device budget {available}"
            );
        }
    }

    #[test]
    fn budget_is_monotonic_bounded_and_keeps_headroom() {
        let mut previous = 0;
        for available in 1..=1024 {
            let budget = cpu_embedding_threads(available);
            assert!(budget >= previous && budget <= available && budget > 0);
            if available > 2 {
                assert!(budget < available);
            }
            previous = budget;
        }
        assert!(cpu_embedding_threads(usize::MAX) <= usize::MAX);
    }
}
