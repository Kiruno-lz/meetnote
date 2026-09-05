use audio::{read_audio_mono_16k, AudioChunkReader, TARGET_SAMPLE_RATE_HZ};

#[test]
#[ignore = "requires MEETNOTE_AUDIO_EQUIVALENCE"]
fn compressed_audio_stream_matches_the_whole_file_baseline() {
    let path = std::env::var("MEETNOTE_AUDIO_EQUIVALENCE")
        .expect("set MEETNOTE_AUDIO_EQUIVALENCE to a compressed audio fixture");
    let baseline = read_audio_mono_16k(&path).unwrap();
    let mut reader = AudioChunkReader::open(&path, TARGET_SAMPLE_RATE_HZ as usize * 7).unwrap();
    let mut streamed = Vec::new();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        assert_eq!(chunk.start_sample, streamed.len() as u64);
        streamed.extend(chunk.samples);
    }

    assert_eq!(streamed.len(), baseline.samples.len());
    let maximum_error = streamed
        .iter()
        .zip(&baseline.samples)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0f32, f32::max);
    assert!(
        maximum_error <= 1e-6,
        "maximum sample error was {maximum_error}"
    );
}
