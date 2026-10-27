/// Match segmentation's nonempty padded short window and overlapping long tail.
pub(crate) fn streaming_total_windows(
    audio_len: usize,
    window_samples: usize,
    step_samples: usize,
) -> usize {
    if audio_len <= window_samples {
        return usize::from(audio_len > 0);
    }
    let full_windows = (audio_len - window_samples) / step_samples + 1;
    let offset_after_full = full_windows * step_samples;
    full_windows + usize::from(offset_after_full < audio_len)
}
