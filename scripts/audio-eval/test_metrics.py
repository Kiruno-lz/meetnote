import unittest

from metrics import diarization, edit_distance, score_record, tokens


class MetricsTests(unittest.TestCase):
    def test_tokenization_and_edit_distance(self):
        self.assertEqual(tokens("你好 世界", "zh"), ["你", "好", "世", "界"])
        self.assertEqual(tokens("Hello, world!", "en"), ["hello", ",", "world", "!"])
        self.assertEqual(edit_distance(["a", "b"], ["a", "c"]), 1)

    def test_perfect_asr_and_diarization_after_label_permutation(self):
        reference = {
            "meetingId": "m1",
            "language": "en",
            "transcript": [{"startMs": 0, "endMs": 1000, "speaker": "A", "text": "Hello"}],
            "referenceSpeakerSegments": [{"startMs": 0, "endMs": 1000, "speaker": "A"}],
        }
        prediction = {
            "schemaVersion": 2,
            "predictionKind": "audio_host_prediction",
            "sourceDomain": "audio",
            "assistantSemanticQualityStatus": "not_assessed",
            "meetingId": "m1",
            "transcript": [{"startMs": 0, "endMs": 1000, "speaker": "X", "text": "Hello"}],
            "speakerSegments": [{"startMs": 0, "endMs": 1000, "speaker": "X"}],
        }
        result = score_record(reference, prediction)
        self.assertEqual(result["asr"]["werOrCer"], 0)
        self.assertEqual(result["diarization"]["der"], 0)

    def test_diarization_counts_false_alarm(self):
        reference = [{"startMs": 0, "endMs": 1000, "speaker": "A"}]
        prediction = [
            {"startMs": 0, "endMs": 1000, "speaker": "X"},
            {"startMs": 1000, "endMs": 1500, "speaker": "X"},
        ]
        result = diarization(reference, prediction)
        self.assertGreater(result["falseAlarmRate"], 0)
        self.assertGreater(result["der"], 0)


if __name__ == "__main__":
    unittest.main()
