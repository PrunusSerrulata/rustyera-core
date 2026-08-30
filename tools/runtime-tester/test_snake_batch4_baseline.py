from __future__ import annotations

import io
import gzip
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest


TOOL_ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOL_ROOT))

import snake_batch4_baseline as baseline  # noqa: E402


class JsonStreamTests(unittest.TestCase):
    def test_reads_arrays_across_tiny_chunks(self) -> None:
        source = io.StringIO(
            '{"skip":[0],"pipeline":{"diagnostics":['
            '{"id":1,"message":"中"},{"id":2}]},"rows":['
            '{"appearance":{"api":"CALL"}},'
            '{"appearance":{"api":"PRINT"}}],"parser_functions":['
            '{"id":0,"name":"TITLE"}]}'
        )
        stream = baseline.JsonStream(source, chunk_size=7)
        stream.seek('"pipeline":')
        stream.seek('"diagnostics":[')
        self.assertEqual([1, 2], [item["id"] for item in stream.array()])
        stream.seek('"rows":[')
        self.assertEqual(
            ["CALL", "PRINT"],
            [item["appearance"]["api"] for item in stream.array()],
        )
        stream.seek('"parser_functions":[')
        self.assertEqual("TITLE", next(stream.array())["name"])


class TargetTests(unittest.TestCase):
    def target(self, *segments: tuple[str, str]) -> dict:
        return {
            "namespace": "function",
            "pattern": {
                "segments": [
                    {"kind": kind, "value": value} for kind, value in segments
                ]
            },
        }

    def test_exact_pattern_requires_only_literals(self) -> None:
        self.assertEqual(
            "GRAPH_DB_INIT",
            baseline.exact_pattern(self.target(("literal", "GRAPH_"), ("literal", "DB_INIT"))),
        )
        self.assertIsNone(
            baseline.exact_pattern(self.target(("literal", "MAP_"), ("unknown", "runtime")))
        )

    def test_dynamic_pattern_is_ascii_case_insensitive_and_ordered(self) -> None:
        target = self.target(
            ("literal", "CAN_"),
            ("unknown", "formatted"),
            ("literal", "_FOR_GRAPH"),
        )
        self.assertTrue(baseline.pattern_matches(target, "can_move_5_for_graph"))
        self.assertFalse(baseline.pattern_matches(target, "move_can_5_for_graph"))


class DiagnosticTests(unittest.TestCase):
    def test_only_overlapping_errors_are_cascade_candidates(self) -> None:
        functions = {
            7: {"id": 7, "name": "TITLE", "path": "ERB/TITLE.ERB", "span": {"start": 0, "end": 100}}
        }
        diagnostics = [
            {"stage": "analyzer", "path": "ERB/TITLE.ERB", "span": {"start": 10, "end": 12}, "code": "Root", "error": True},
            {"stage": "analyzer", "path": "ERB/TITLE.ERB", "span": {"start": 11, "end": 13}, "code": "Overlap", "error": True},
            {"stage": "analyzer", "path": "ERB/TITLE.ERB", "span": {"start": 20, "end": 21}, "code": "Independent", "error": True},
            {"stage": "analyzer", "path": None, "span": None, "code": "Unlocated", "error": True},
            {"stage": "analyzer", "path": "ERB/TITLE.ERB", "span": {"start": 30, "end": 31}, "code": "Warning", "error": False},
        ]
        classified, counts = baseline.diagnostic_classification(
            diagnostics, functions, {"title": {7}}, {"title": set()}
        )
        self.assertEqual("first_root_at_overlapping_span", classified[0]["rootStatus"])
        self.assertEqual("cascade_candidate_overlapping_span", classified[1]["rootStatus"])
        self.assertEqual("independent_error", classified[2]["rootStatus"])
        self.assertEqual("independent_unlocated_error", classified[3]["rootStatus"])
        self.assertEqual("non_error", classified[4]["rootStatus"])
        self.assertEqual(4, counts["static_reachable"])
        self.assertEqual(1, counts["unlocated"])


class FixtureTests(unittest.TestCase):
    def test_contract_and_resource_inputs_are_frozen(self) -> None:
        plan = baseline.read_object(baseline.PLAN_PATH)
        contracts = baseline.read_object(baseline.FIXTURE_ROOT / "contracts.json")
        self.assertEqual(baseline.SEMANTIC_BASELINE, plan["semanticBaseline"])
        self.assertEqual([2, 6, 8, 10], contracts["contracts"]["sprite"]["arities"])
        self.assertFalse(contracts["bbas"]["knownMissing"] == [])
        manifest = baseline.fixture_manifest(plan)
        external = [item for item in manifest if item["path"].startswith("external:")]
        self.assertEqual(2, len(external))
        paths = {item["path"] for item in manifest}
        self.assertNotIn("README.md", paths)
        self.assertNotIn("contracts.json", paths)

    def test_resource_destination_cannot_escape_disposable_game(self) -> None:
        plan = baseline.read_object(baseline.PLAN_PATH)
        plan["resourceDependencies"][0]["destination"] = "../escape.png"
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                baseline.assemble_fixture(Path(directory), plan)

    def test_load_request_binds_the_fixed_font(self) -> None:
        class FakeOracle:
            @staticmethod
            def windows_path(path: Path) -> str:
                return "W:" + path.as_posix()

        plan = baseline.read_object(baseline.PLAN_PATH)
        with tempfile.TemporaryDirectory() as directory:
            game = baseline.assemble_fixture(Path(directory), plan)
            request = baseline.load_request(FakeOracle(), game, plan)
        self.assertTrue(request["observePresentation"])
        self.assertEqual("BIZ UDGothic", request["presentationFont"]["family"])
        self.assertEqual(
            "e267830408f04daf92858d89477f2df8539c05ee4fe597d13ffdcaa7565b519e",
            request["presentationFont"]["sha256"],
        )

    def test_nonzero_reference_exit_rejects_stable_projection(self) -> None:
        plan = {"cases": [{"id": "one"}], "viewport": {}}
        capture = {
            "formatVersion": 1,
            "semanticBaseline": baseline.SEMANTIC_BASELINE,
            "implementation": baseline.IMPLEMENTATION,
            "referenceExecutable": {},
            "wrapper": {},
            "fixtureManifest": [],
            "cases": {
                "one": {
                    "runnerFailure": None,
                    "processExitCode": 9,
                    "load": {"result": {"presentation": {}}},
                    "run": {"result": {}},
                }
            },
        }
        with self.assertRaises(AssertionError):
            baseline.stable_capture_projection(capture, plan)

class ClassificationIntegrationTests(unittest.TestCase):
    @staticmethod
    def target(dispatch: str, *segments: tuple[str, str]) -> dict:
        return {
            "dispatch": dispatch,
            "namespace": "function",
            "pattern": {"segments": [{"kind": kind, "value": value} for kind, value in segments]},
            "expected_return": None,
            "supplied_slots": 0,
            "omitted_slots": 0,
            "executes_body": True,
        }

    @classmethod
    def row(cls, owner: int, api: str, *, active: bool = True, target: dict | None = None) -> dict:
        return {
            "appearance": {
                "path": "ERB/TITLE.ERB",
                "api": api,
                "activity": "active_ast" if active else "inactive_preprocessor",
                "span_status": "valid_decoded_utf8",
                "ownership_status": "parser_function_membership_not_execution",
                "owning_function": owner,
                "target": target,
            }
        }

    def write_report(self, root: Path) -> tuple[Path, dict]:
        diagnostics = [
            {"stage": "analyzer", "path": "ERB/TITLE.ERB", "span": {"start": 10, "end": 12}, "code": "Root", "error": True},
            {"stage": "analyzer", "path": "ERB/TITLE.ERB", "span": {"start": 11, "end": 13}, "code": "Cascade", "error": True},
            {"stage": "analyzer", "path": "ERB/TITLE.ERB", "span": {"start": 50, "end": 51}, "code": "Independent", "error": True},
        ]
        rows = [
            self.row(0, "CALL", target=self.target("direct_statement", ("literal", "NEXT"))),
            self.row(0, "CALLFORM", target=self.target("dynamic_statement", ("literal", "NEXT"), ("unknown", "suffix"))),
            self.row(1, "PRINT"),
            self.row(1, "INACTIVE_ONLY", active=False),
        ]
        functions = [
            {"id": 0, "name": "TITLE", "path": "ERB/TITLE.ERB", "span": {"start": 0, "end": 100}},
            {"id": 1, "name": "NEXT", "path": "ERB/TITLE.ERB", "span": {"start": 100, "end": 200}},
        ]
        value = {
            "kind": "snake_compatibility_static_coverage",
            "projects": [{
                "project": "fixture",
                "profile_override": "emuera.skia.snake",
                "pipeline": {"diagnostics": diagnostics},
                "rows": rows,
                "parser_functions": functions,
            }],
        }
        raw = (json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
        report = root / "coverage.json.gz"
        with gzip.GzipFile(filename=report, mode="wb", mtime=0) as output:
            output.write(raw)
        stored = report.read_bytes()
        expected = {
            "kind": "snake_compatibility_static_coverage",
            "project": "fixture",
            "profile": "emuera.skia.snake",
            "diagnostics": 3,
            "storedFile": {"bytes": len(stored), "sha256": hashlib.sha256(stored).hexdigest()},
            "rawJson": {"bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()},
        }
        manifest = {
            "status": "complete",
            "stored_file": {"blake3": "fixture", **expected["storedFile"]},
            "raw_json": {"blake3": "fixture", **expected["rawJson"]},
        }
        Path(str(report) + ".manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        return report, expected

    def test_end_to_end_stream_classification_and_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            report, expected = self.write_report(root)
            output = root / "classification.json"
            baseline.classify_report(
                report,
                output,
                {"title": ["TITLE", "MISSING"]},
                expected,
            )
            result = baseline.read_object(output)
            route = result["routes"]["title"]
            self.assertEqual(["MISSING"], route["missingSeeds"])
            self.assertEqual(2, route["staticFunctionCount"])
            self.assertEqual({"CALL": 1, "CALLFORM": 1, "PRINT": 1}, route["reachableApiCounts"])
            self.assertNotIn("INACTIVE_ONLY", route["reachableApiCounts"])
            self.assertEqual(1, len(route["dynamicTargets"]))
            self.assertEqual(
                [0, 2], result["implementationCandidateDiagnosticIds"]
            )
            self.assertFalse(Path(str(output) + ".work.sqlite3").exists())

    def test_stored_identity_mismatch_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "coverage.json.gz"
            report.write_bytes(b"not-the-frozen-report")
            expected = {"storedFile": {"bytes": 1, "sha256": "0" * 64}}
            with self.assertRaises(AssertionError):
                baseline.validate_report_identity(report, expected)


if __name__ == "__main__":
    unittest.main()
