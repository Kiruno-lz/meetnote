//! Deterministic alignment resource probe; no ASR/DIA quality claim.
//! Usage: alignment_benchmark <short|long|nested> <repeats> [max_median_ms]
use domain::{
    align_transcript_and_speakers, DiarizationSegment, SpeakerTaggedSegment, TranscriptSegment,
};
use std::{hint::black_box, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        return Err("expected <short|long|nested> <repeats> [max_median_ms]".into());
    }
    let turns = match args[0].as_str() {
        "short" => 32,
        "long" | "nested" => 16_000,
        _ => return Err("unknown fixture".into()),
    };
    let repeats: usize = args[1].parse()?;
    let maximum: Option<f64> = args.get(2).map(|value| value.parse()).transpose()?;
    if repeats == 0 || maximum.is_some_and(|value| !value.is_finite() || value <= 0.0) {
        return Err("repeats and optional maximum must be positive and finite".into());
    }
    let nested = args[0] == "nested";
    let mut transcript = Vec::new();
    let mut diarization = Vec::new();
    let mut gold = Vec::new();
    for turn in 0..turns {
        let speaker = format!("speaker-{}", turn % 4);
        let profile = Some(format!("profile-{}", turn % 4));
        let label = Some(format!("Speaker {}", turn % 4));
        diarization.push(DiarizationSegment {
            id: format!("d{turn}"),
            speaker_id: speaker.clone(),
            speaker_profile_id: profile.clone(),
            speaker_label: label.clone(),
            start_ms: turn * 4500,
            end_ms: (turn + 1) * 4500,
            confidence: None,
        });
        for piece in 0..3 {
            let segment = TranscriptSegment {
                id: format!("t{turn}-{piece}"),
                start_ms: turn * 4500 + piece * 1500,
                end_ms: turn * 4500 + (piece + 1) * 1500,
                text: "fixed fixture text".into(),
            };
            gold.push(SpeakerTaggedSegment {
                transcript_segment_id: segment.id.clone(),
                speaker_id: if nested {
                    "long-speaker".into()
                } else {
                    speaker.clone()
                },
                speaker_profile_id: if nested { None } else { profile.clone() },
                speaker_label: if nested { None } else { label.clone() },
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                text: segment.text.clone(),
            });
            transcript.push(segment);
        }
    }
    // Unsorted input and one containing interval must not defeat the index.
    diarization.reverse();
    if nested {
        diarization.push(DiarizationSegment {
            id: "containing-interval".into(),
            speaker_id: "long-speaker".into(),
            speaker_profile_id: None,
            speaker_label: None,
            start_ms: 0,
            end_ms: turns * 4500,
            confidence: None,
        });
    }
    let mut milliseconds = Vec::new();
    for _ in 0..repeats {
        let started = Instant::now();
        let result = black_box(align_transcript_and_speakers(
            black_box(&transcript),
            black_box(&diarization),
        ));
        milliseconds.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(
            result, gold,
            "alignment changed speaker, identity, text, range or order"
        );
    }
    let mut sorted = milliseconds.clone();
    sorted.sort_by(f64::total_cmp);
    let median = (sorted[(repeats - 1) / 2] + sorted[repeats / 2]) / 2.0;
    let passed = maximum.is_none_or(|value| median <= value);
    println!(
        "{}",
        serde_json::json!({
            "schema": "meetnote.alignment-resource.v1", "fixture": args[0],
            "scope": "synthetic interval resource and exact assignment, not model quality",
            "transcriptSegments": transcript.len(), "diarizationSegments": diarization.len(),
            "timelineMs": turns * 4500, "repeats": repeats,
            "milliseconds": milliseconds, "medianMs": median, "maximumMedianMs": maximum,
            "exactGoldMatch": true, "status": if passed { "passed" } else { "failed" },
        })
    );
    if !passed {
        return Err("alignment exceeded the frozen local latency gate".into());
    }
    Ok(())
}
