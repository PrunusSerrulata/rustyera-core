"""Offline scope, corruption and binary-boundary regressions for the release gate."""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import snake_release_gate as gate


class ReleaseGateTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "source.rs").write_text("pub struct Standard1808;\n")
        (self.root / "release.wasm").write_bytes(b"\x00asm\x01\x00\x00\x00")
        self.manifest = {
            "sources": [{"path": "source.rs"}],
            "artifacts": [{"path": "release.wasm", "role": "wasm", "build_identity": "test receipt"}],
        }

    def inspect(self):
        path = self.root / "manifest.json"
        path.write_text(json.dumps(self.manifest))
        return gate.inspect_manifest(path)

    def test_clean_inputs_are_bound_to_content(self):
        report, code = self.inspect()
        self.assertEqual(code, 0)
        self.assertEqual(report["status"], "clean")
        self.assertEqual(report["inputs"][1]["sha256"], hashlib.sha256(
            (self.root / "release.wasm").read_bytes()).hexdigest())

    def test_missing_artifact_or_empty_scope_cannot_pass(self):
        (self.root / "release.wasm").unlink()
        self.assertEqual(self.inspect()[1], 2)
        self.manifest["artifacts"] = []
        self.assertEqual(self.inspect()[1], 2)

    def test_binary_matches_cross_chunks_and_cover_utf16(self):
        marker = b"RERASAV"
        (self.root / "release.wasm").write_bytes(b"_" * 14 + marker + marker.decode().encode("utf-16-le"))
        with patch.object(gate, "CHUNK_BYTES", 16):
            report, code = self.inspect()
        self.assertEqual(code, 1)
        findings = report["inputs"][1]["findings"]
        self.assertEqual({(item["encoding"], item["count"], tuple(item["offsets"]))
                          for item in findings}, {("utf-8", 1, (14,)), ("utf-16-le", 1, (21,))})

    def test_exact_test_exclusion_does_not_hide_production_match(self):
        production = b'const BAD: &str = "RERASAV";\n'
        tests = b'#[cfg(test)] mod tests { const INPUT: &str = "RERASAV"; }\n'
        (self.root / "source.rs").write_bytes(production + tests)
        self.manifest["sources"][0]["excluded_ranges"] = [{
            "start": len(production), "end": len(production + tests),
            "sha256": hashlib.sha256(tests).hexdigest(), "reason": "Entire cfg(test) module",
        }]
        report, code = self.inspect()
        self.assertEqual(code, 1)
        self.assertEqual(report["inputs"][0]["findings"][0]["count"], 1)
        (self.root / "source.rs").write_bytes(production + tests.replace(b"INPUT", b"OTHER"))
        self.assertEqual(self.inspect()[1], 2)

    def test_explicit_test_file_exclusion_and_duplicate_rejection(self):
        (self.root / "tests.rs").write_text('const INPUT: &str = "RERASAV";')
        self.manifest["excluded"] = [{"path": "tests.rs", "reason": "Negative fixture"}]
        self.assertEqual(self.inspect()[1], 0)
        self.manifest["excluded"].append({"path": "source.rs", "reason": "Wrong duplicate"})
        self.assertEqual(self.inspect()[1], 2)

    def test_release_file_exclusions_are_forbidden(self):
        self.manifest["artifacts"][0]["excluded_ranges"] = []
        self.assertEqual(self.inspect()[1], 2)

    def test_legacy_namespace_wire_name_is_rejected(self):
        (self.root / "release.wasm").write_bytes(b'"namespace":"legacy_profile_save"')
        report, code = self.inspect()
        self.assertEqual(code, 1)
        self.assertEqual(report["inputs"][1]["findings"][0]["marker"], "legacy_profile_save")


if __name__ == "__main__":
    unittest.main()
