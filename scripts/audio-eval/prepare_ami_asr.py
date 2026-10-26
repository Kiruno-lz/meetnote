#!/usr/bin/env python3
"""Build oracle-cropped, non-overlapping AMI conversational B1 cases, not DIA gold."""
import argparse
import hashlib
import json
import re
import wave
import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path
from decimal import Decimal, DecimalException, ROUND_HALF_EVEN

AUDIO_SHA = "9c76866990fcc8b84006dc32d273ad99df439090b748ebe72103bb78c3216ee7"
ANNOTATION_SHA = "b56e5babb2496b8795deeeda7e71178d7fbc9963f94276cf2a3f4b56ebbc9f9d"
NITE = "{http://nite.sourceforge.net/}"


def sha256(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def project_reference_words(words, first, last):
    """Project official forced-alignment times; never infer silence or clip words."""
    if type(first) is not int or type(last) is not int or not 0 <= first < last:
        raise ValueError("invalid crop sample range")
    projected = []
    seen = set()
    previous_start = Decimal(-1)
    for word in words:
        identity = word.attrib.get(NITE + "id", "")
        if not identity or identity in seen or not (word.text or "").strip():
            raise ValueError(f"invalid or duplicate lexical word: {identity}")
        seen.add(identity)
        try:
            start = Decimal(word.attrib["starttime"])
            end = Decimal(word.attrib["endtime"])
            if not start.is_finite() or not end.is_finite() or not 0 <= start < end:
                raise ValueError(f"invalid word time: {identity}")
            start *= 16000
            end *= 16000
        except (DecimalException, KeyError) as error:
            raise ValueError(f"missing or invalid word time: {identity}") from error
        if not first <= start < end <= last or start < previous_start:
            raise ValueError(f"word crosses crop or is out of order: {identity}")
        previous_start = start
        start_sample = int(start.to_integral_value(rounding=ROUND_HALF_EVEN)) - first
        end_sample = int(end.to_integral_value(rounding=ROUND_HALF_EVEN)) - first
        if start_sample >= end_sample:
            raise ValueError(f"word duration is below sample resolution: {identity}")
        projected.append(
            {
                "id": identity,
                "text": word.text,
                "startSample": start_sample,
                "endSample": end_sample,
            }
        )
    if not projected:
        raise ValueError("no lexical word times")
    return {
        "source": "ami-manual-1.6.2-forced-alignment",
        "sampleRateHz": 16000,
        "manuallyVerified": False,
        "words": projected,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--audio", type=Path, required=True)
    parser.add_argument("--annotations", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    if sha256(args.audio) != AUDIO_SHA or sha256(args.annotations) != ANNOTATION_SHA:
        raise ValueError("AMI source checksum mismatch")
    words = {}
    segments = {}
    with zipfile.ZipFile(args.annotations) as archive:
        for speaker in "ABCD":
            tree = ET.fromstring(archive.read(f"words/ES2002a.{speaker}.words.xml"))
            words[speaker] = [w for w in tree if w.tag == "w" and "starttime" in w.attrib]
            segments[speaker] = ET.fromstring(
                archive.read(f"segments/ES2002a.{speaker}.segments.xml")
            )
    candidates = []
    for speaker in "ABCD":
        all_words = words[speaker]
        by_id = {w.attrib[NITE + "id"]: i for i, w in enumerate(all_words)}
        selected = 0
        for segment in segments[speaker]:
            href = next(iter(segment)).attrib["href"]
            ids = re.findall(r"id\(([^)]+)\)", href)
            if not ids or ids[0] not in by_id or ids[-1] not in by_id:
                continue
            span = [
                w
                for w in all_words[by_id[ids[0]] : by_id[ids[-1]] + 1]
                if w.attrib.get("punc") != "true"
            ]
            if len(span) < 6:
                continue
            start = min(float(w.attrib["starttime"]) for w in span) - 0.1
            end = max(float(w.attrib["endtime"]) for w in span) + 0.1
            if not 3 <= end - start <= 20:
                continue
            # Reject any other speaker's lexical overlap, including the padding.
            if any(
                float(w.attrib["starttime"]) < end and float(w.attrib["endtime"]) > start
                for other in "ABCD"
                if other != speaker
                for w in words[other]
                if w.attrib.get("punc") != "true"
            ):
                continue
            candidates.append((speaker, start, end, span))
            selected += 1
            if selected == 2:
                break
        if selected == 0:
            raise ValueError(f"no unambiguous case for {speaker}")
    root = args.root.resolve()
    (root / "audio").mkdir(parents=True, exist_ok=True)
    cases = []
    with wave.open(str(args.audio), "rb") as source:
        if (source.getnchannels(), source.getsampwidth(), source.getframerate()) != (1, 2, 16000):
            raise ValueError("unexpected AMI signal format")
        prepared = []
        for i, (speaker, start, end, span) in enumerate(sorted(candidates, key=lambda x: x[1])):
            first = max(0, round(start * 16000))
            last = round(end * 16000)
            if last > source.getnframes():
                raise ValueError(f"crop exceeds AMI audio: {speaker}")
            try:
                timing = project_reference_words(span, first, last)
            except ValueError as error:
                raise ValueError(
                    f"{args.annotations}: words/ES2002a.{speaker}.words.xml: {error}"
                ) from error
            prepared.append((i, speaker, first, last, span, timing))
        # Validate all references before replacing any derived crop audio.
        for i, speaker, first, last, span, timing in prepared:
            source.setpos(first)
            data = source.readframes(last - first)
            path = root / "audio" / f"ami-ES2002a-{i}.wav"
            with wave.open(str(path), "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(2)
                output.setframerate(16000)
                output.writeframes(data)
            cases.append(
                {
                    "id": f"ami-ES2002a-{i}",
                    "language": "en",
                    "scenario": "ami-oracle-nonoverlap",
                    "audioPath": str(path),
                    "sha256": sha256(path),
                    "reference": " ".join(w.text or "" for w in span),
                    "durationSeconds": len(data) / 32000,
                    "sourceSpeaker": speaker,
                    "startSample": first,
                    "endSample": last,
                    "sourceWordIds": [w.attrib[NITE + "id"] for w in span],
                    "wordTiming": timing,
                }
            )
    (root / "ami-cases.json").write_text(json.dumps(cases, ensure_ascii=False, indent=2) + "\n")
    (root / "ami-receipt.json").write_text(
        json.dumps(
            {
                "audioSource": "https://groups.inf.ed.ac.uk/ami/AMICorpusMirror/amicorpus/ES2002a/audio/ES2002a.Mix-Headset.wav",
                "annotationSource": "https://groups.inf.ed.ac.uk/ami/AMICorpusAnnotations/ami_public_manual_1.6.2.zip",
                "license": "CC-BY-4.0",
                "audioSha256": AUDIO_SHA,
                "annotationsSha256": ANNOTATION_SHA,
                "scope": "up to first two eligible original segments per speaker, at least one per speaker; 3-20s, >=6 lexical words, 100ms padding, no other speaker lexical overlap",
                "diarizationQualityStatus": "not_assessed",
                "wordTimingSource": "official forced alignment; not manually verified word boundaries",
                "casesSha256": sha256(root / "ami-cases.json"),
            },
            indent=2,
        )
        + "\n"
    )
    print(json.dumps({"cases": len(cases), "seconds": sum(c["durationSeconds"] for c in cases)}))


if __name__ == "__main__":
    main()
