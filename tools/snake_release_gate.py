#!/usr/bin/env python3
"""Check explicit production sources and release files for removed snake save code.

Usage: python3 tools/snake_release_gate.py manifest.json > report.json

The manifest is a JSON object with `sources`, `artifacts`, and `excluded` lists.
Every entry has `path`, relative to the manifest (absolute paths also work).
Sources are individual production code, public schema, or C ABI header files.
Artifacts are individual, uncompressed release files with a caller-chosen `role`
and `build_identity` explaining their build receipt/core revision. Application
bundles must be enumerated as files; scanning a compressed archive is insufficient.
At least one source and one artifact are required. The caller owns completeness
and fresh-release provenance; this tool does not discover build inputs or build.

Excluded whole files require `reason`. For inline test code, a source may declare
`excluded_ranges`: [{"start": 100, "end": 200, "sha256": "...", "reason": "..."}].
Offsets are zero-based bytes, end-exclusive. The caller must review these precise
boundaries against cfg(test); this tool verifies bytes, not Rust conditional
compilation. No path, comment, string, or signature is implicitly filtered. In
particular, excluding a test range cannot suppress the same marker elsewhere in
the production file. Keep this checker and its negative fixtures out of sources.

Example: {"sources": [{"path": "src/lib.rs"}], "artifacts": [{"path":
"target/release/library.dylib", "role": "dylib", "build_identity": "receipt.json"}],
"excluded": [{"path": "src/tests.rs", "reason": "cfg(test) module"}]}

Exit 0 means the declared scope is clean; 1 means prohibited bytes were found;
2 means an invalid manifest or missing/unreadable/changed input. JSON includes
input SHA-256, excluded ranges, all signature counts and bounded match offsets.
"""

import argparse
import hashlib
import json
from pathlib import Path
import sys


# Exact removed symbols/bytes from core commit 6257601554b0123b3e06eada617fe965fc687d4a^.
PROHIBITED = (
    "RERASAV",
    "rustyera.rerasav.envelope.v2",
    "rustyera_envelope_v2:emuera1808",
    "rustyera.save_state",
    "LEGACY_SNAKE_OWNED_SAVE_CODEC",
    "SAVE_STATE_CONTRACT_NAME",
    "SAVE_STATE_CONTRACT_VERSION",
    "LegacyProfileSave",
    "legacy_profile_save",
    "LegacySnakeOwnedV11",
    "LegacyRustyEraOwnedV11",
    "OwnedSaveStateV1",
    "DecodedOwnedSaveState",
    "OwnedDatabaseRevisionV1",
    "CompatibleSaveEnvelope",
    "CompatibleSaveSource",
    "legacy_snake_owned_save_v11",
    "unwrap_compatible_envelope",
    "unwrap_compatible_save",
    "inspect_compatible_metadata",
    "envelope_checksum",
    "envelope_prefix",
    "decode_owned_state",
    "preflight_owned_state",
    "encode_owned_era_save",
)
PATTERNS = tuple((name, encoding, name.encode(encoding))
                 for name in PROHIBITED for encoding in ("utf-8", "utf-16-le", "utf-16-be"))
CHUNK_BYTES = 1024 * 1024
MAX_OFFSETS = 20


def required_text(entry, key):
    value = entry.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{key} must be a nonempty string")
    return value


def excluded_ranges(entry, data):
    ranges = entry.get("excluded_ranges", [])
    if not isinstance(ranges, list):
        raise ValueError("excluded_ranges must be a list")
    previous_end = 0
    for item in ranges:
        start, end = item["start"], item["end"]
        if (type(start) is not int or type(end) is not int
                or not previous_end <= start < end <= len(data)):
            raise ValueError("excluded_ranges must be ordered, disjoint, nonempty byte ranges")
        required_text(item, "reason")
        expected = required_text(item, "sha256")
        if hashlib.sha256(data[start:end]).hexdigest() != expected:
            raise ValueError(f"excluded range [{start}, {end}) SHA-256 differs")
        previous_end = end
    return ranges


def scan_file(path, ranges):
    """Stream raw files and preserve matches that cross read boundaries."""
    digest = hashlib.sha256()
    findings = {}
    overlap = max(len(pattern) for _, _, pattern in PATTERNS) - 1
    tail, total = b"", 0
    with path.open("rb") as stream:
        while chunk := stream.read(CHUNK_BYTES):
            digest.update(chunk)
            window, base = tail + chunk, total - len(tail)
            for marker, encoding, pattern in PATTERNS:
                position = window.find(pattern)
                while position >= 0:
                    start, end = base + position, base + position + len(pattern)
                    # Matches ending in the old tail were already inspected.
                    if end > total and not any(
                            item["start"] <= start and end <= item["end"] for item in ranges):
                        finding = findings.setdefault((marker, encoding), {
                            "marker": marker, "encoding": encoding, "count": 0, "offsets": []})
                        finding["count"] += 1
                        if len(finding["offsets"]) < MAX_OFFSETS:
                            finding["offsets"].append(start)
                    position = window.find(pattern, position + 1)
            total += len(chunk)
            tail = window[-overlap:]
    return {"sha256": digest.hexdigest(), "bytes": total,
            "findings": list(findings.values())}


def inspect_entry(entry, kind, manifest_dir):
    path = (manifest_dir / required_text(entry, "path")).resolve(strict=True)
    if not path.is_file():
        raise ValueError(f"expected an individual regular file: {path}")
    if kind != "source" and "excluded_ranges" in entry:
        raise ValueError("excluded_ranges are only allowed for source files")
    result = {"path": str(path), "kind": kind}
    if kind == "excluded":
        result["reason"] = required_text(entry, "reason")
        with path.open("rb") as stream:
            result["sha256"] = hashlib.file_digest(stream, "sha256").hexdigest()
        result["bytes"] = path.stat().st_size
        return result
    if kind == "artifact":
        result["role"] = required_text(entry, "role")
        result["build_identity"] = required_text(entry, "build_identity")
    ranges = []
    source_digest = None
    if kind == "source" and "excluded_ranges" in entry:
        data = path.read_bytes()
        source_digest = hashlib.sha256(data).hexdigest()
        ranges = excluded_ranges(entry, data)
        result["excluded_ranges"] = ranges
    scanned = scan_file(path, ranges)
    if not scanned["bytes"]:
        raise ValueError(f"empty {kind} input: {path}")
    if source_digest is not None and scanned["sha256"] != source_digest:
        raise ValueError(f"source changed while inspecting excluded ranges: {path}")
    return {**result, **scanned}


def inspect_manifest(manifest_path):
    report = {"schema_version": 1, "status": "error", "inputs": [], "errors": [],
              "scope": "Explicit files only; source completeness and fresh-release provenance require caller review.",
              "prohibited": list(PROHIBITED)}
    try:
        manifest_bytes = manifest_path.read_bytes()
        report["manifest_sha256"] = hashlib.sha256(manifest_bytes).hexdigest()
        manifest = json.loads(manifest_bytes)
        if not isinstance(manifest, dict):
            raise ValueError("manifest must be a JSON object")
        for group in ("sources", "artifacts", "excluded"):
            entries = manifest.get(group, [])
            if not isinstance(entries, list) or (group != "excluded" and not entries):
                raise ValueError(f"{group} must be a list" + ("" if group == "excluded" else " with at least one entry"))
        seen = set()
        for group, kind in (("sources", "source"), ("artifacts", "artifact"), ("excluded", "excluded")):
            for entry in manifest.get(group, []):
                try:
                    result = inspect_entry(entry, kind, manifest_path.parent)
                    if result["path"] in seen:
                        raise ValueError(f"duplicate input: {result['path']}")
                    seen.add(result["path"])
                    report["inputs"].append(result)
                except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
                    report["errors"].append({"kind": kind, "entry": entry, "message": str(error)})
    except (OSError, ValueError, TypeError) as error:
        report["errors"].append({"message": str(error)})
    if report["errors"]:
        return report, 2
    found = any(item.get("findings") for item in report["inputs"])
    report["status"] = "prohibited_bytes" if found else "clean"
    return report, 1 if found else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    report, code = inspect_manifest(args.manifest.resolve())
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return code


if __name__ == "__main__":
    sys.exit(main())
