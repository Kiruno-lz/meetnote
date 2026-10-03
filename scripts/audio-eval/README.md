# Audio evaluation boundary

The files in this directory evaluate Audio Host evidence only. They cover
dataset ASR/diarization references and the composed-audio boundary smoke case.
They do not evaluate Assistant decisions such as semantic topic boundaries,
titles, summaries, tags, or document-tree structure.

## Artifact contract

Audio predictions must use an explicit envelope:

```json
{
  "schemaVersion": 2,
  "predictionKind": "audio_host_prediction",
  "sourceDomain": "audio",
  "assistantSemanticQualityStatus": "not_assessed",
  "segments": []
}
```

`assistant_semantic_prediction` and legacy untyped segment payloads are rejected
by the composite evaluator and the Audio Host metrics. A report may contain
`audio_boundary_scored` or `audio_metrics_scored`, but Assistant semantic
quality remains explicitly `not_assessed`.

An application-side Audio Host export in an older shape must be adapted to this
envelope before scoring. The scorer intentionally rejects predictions that carry
`referenceSpeakerSegments`: reference annotations are gold input, never model
output, and using them would produce an invalidly optimistic DER.

The composed-audio reference is a boundary fixture. Its clip `topic` values
identify fixture construction only; they are not Assistant semantic gold
labels. A passing boundary recall therefore proves only that the Audio Host
prediction carried the expected temporal boundary, not that an Assistant made
the correct semantic split.

Targeted checks:

```sh
PYTHONPATH=scripts/audio-eval:scripts/assistant-eval \
  python3 -m unittest scripts/audio-eval/test_metrics.py scripts/audio-eval/test_boundary_contract.py
```
