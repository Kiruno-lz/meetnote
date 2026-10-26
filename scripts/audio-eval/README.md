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

DER uses the optional, pinned `requirements-diarization.txt` environment
(`pyannote.metrics` 4.0.0 and `pyannote.core` 6.1.0). It uses optimal Hungarian
label mapping, includes speaker overlap and coalesces repeated intervals of the
same speaker. `--collar-ms` is the full duration centered on each reference
boundary, excluded from both timelines. `durationMs` bounds scoring when supplied;
otherwise the report explicitly marks `observed_extent`, which is not a full
recording/UEM acceptance claim.

Without the scorer or `speakerSegments`, DER is `unavailable` with a null rate.
Zero reference speaker time, including after collar exclusion, is unscored rather
than perfect. Explicit empty hypotheses against reference speech still score as
misses. The report is `audio_metrics_partial` if any DER is unscored; no hand-built
DER fallback is used. The dependency file explicitly includes the undeclared
`typing_extensions` import required by this release.

Direct scoring also rejects a supplied prediction meeting ID that differs from
its reference. The CLI rejects empty, duplicate, missing and unexpected meeting IDs, protects
input paths and hardlinks, hashes the exact input snapshots used for scoring, and
publishes reports atomically. Failed reruns remove stale reports. Numerical
regressions require the optional environment; the regular Rust export protocol
also checks the explicit unavailable case. Neither fixture validation nor export
success proves a model's DER. See the [validation receipt](../../docs/quality/der-scorer-validation.json).

Targeted checks:

```sh
PYTHONPATH=scripts/audio-eval:scripts/assistant-eval \
  python3 -m unittest scripts/audio-eval/test_metrics.py scripts/audio-eval/test_boundary_contract.py
```
