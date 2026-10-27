use speakrs::inference::SegmentationError;

// Test the actual private vendor planner, including its existing tensor checks.
#[path = "../../../third_party/speakrs/src/inference/segmentation/tensor.rs"]
mod tensor;

#[test]
fn short_nonempty_audio_has_one_zero_padded_inference_window() {
    for len in [1, 4_000, 159_999] {
        let audio = vec![0.25; len];
        let windows = tensor::SegmentationWindows::collect(&audio, 160_000, 16_000);
        assert_eq!(windows.total_windows(), 1, "{len} samples were omitted");
        let window = windows.window(0, "short recording").unwrap();
        assert_eq!(window.len(), 160_000);
        assert_eq!(&window[..len], &audio);
        assert!(window[len..].iter().all(|sample| *sample == 0.0));
        assert!(windows.window(1, "out of range").is_err());
    }
}

#[test]
fn empty_audio_does_not_create_an_inference_window() {
    let windows = tensor::SegmentationWindows::collect(&[], 160_000, 16_000);
    assert!(windows.is_empty());
    assert!(windows.window(0, "empty recording").is_err());
}

#[test]
fn complete_single_window_does_not_add_a_padded_copy() {
    let audio = vec![0.5; 160_000];
    let windows = tensor::SegmentationWindows::collect(&audio, 160_000, 16_000);
    assert_eq!(windows.total_windows(), 1);
    assert_eq!(windows.window(0, "complete recording").unwrap(), &audio);
}

#[test]
fn longer_unaligned_recordings_keep_their_existing_windows() {
    for (len, count) in [(11, 2), (20, 5)] {
        let audio: Vec<f32> = (0..len).map(|sample| sample as f32 + 1.0).collect();
        let windows = tensor::SegmentationWindows::collect(&audio, 10, 3);
        assert_eq!(windows.total_windows(), count);
        for index in 0..count {
            let window = windows.window(index, "longer recording").unwrap();
            let first = index * 3;
            let remaining = (len - first).min(10);
            assert_eq!(&window[..remaining], &audio[first..first + remaining]);
            assert!(window[remaining..].iter().all(|sample| *sample == 0.0));
        }
    }
}

#[path = "../../../third_party/speakrs/src/inference/segmentation/window_count.rs"]
mod window_count;

#[test]
fn concurrent_allocation_matches_producer_at_every_short_boundary() {
    for window in [10, 160] {
        for step in [1, 3, window] {
            for len in 0..=window * 3 {
                let audio = vec![0.25; len];
                let windows = tensor::SegmentationWindows::collect(&audio, window, step);
                assert_eq!(
                    window_count::streaming_total_windows(len, window, step),
                    windows.total_windows(),
                    "len={len} window={window} step={step}"
                );
            }
        }
    }
}
