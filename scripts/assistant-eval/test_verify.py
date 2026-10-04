#!/usr/bin/env python3

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from evaluate import read_jsonl
from verify import MODEL_MATRIX, verify_model_matrix, verify_oracle_regression


ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "crates/application/tests/fixtures/assistant/synthetic-corpus.jsonl"
REVIEW_MANIFEST = ROOT / "scripts/assistant-eval/review-manifest.json"


class MatrixTests(unittest.TestCase):
    def test_model_matrix_has_all_protocols_and_cases(self):
        verify_model_matrix()
        matrix = json.loads(MODEL_MATRIX.read_text(encoding="utf-8"))
        self.assertEqual(len(matrix["protocols"]), 3)
        self.assertEqual(len(matrix["cases"]), 10)

    def test_oracle_regression_is_explicitly_not_quality(self):
        records = read_jsonl(CORPUS)
        manifest = json.loads(REVIEW_MANIFEST.read_text(encoding="utf-8"))
        result = verify_oracle_regression(
            records, [sample["scenarioId"] for sample in manifest["samples"]]
        )
        self.assertEqual(result["status"], "structural_regression")
        self.assertEqual(result["predictionSource"], "oracle-gold-copy")
        self.assertEqual(result["sampleCount"], 24)


if __name__ == "__main__":
    unittest.main()
