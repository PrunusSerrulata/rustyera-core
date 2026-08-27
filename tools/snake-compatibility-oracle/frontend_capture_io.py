"""Bounded, identity-preserving readers for real frontend observations.

This module never launches a browser, modifies a fixture or supplies a service.
"""

import codecs
import gzip
import hashlib
import io
import json
import math
import fnmatch
import re
import stat
import unicodedata
from pathlib import Path


MIB = 1024 * 1024
MAX_MANIFEST = 4 * MIB
MAX_RECORD = 32 * MIB
MAX_PAYLOAD = 4 * MIB
MAX_TRACE = 512 * MIB
MAX_STORED = 1024 * MIB
MAX_SOURCE = 64 * MIB
MAX_FILES = 100_000
MAX_NODES = 4_194_304


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, minimum=0, maximum=(1 << 64) - 1):
    if isinstance(value, str):
        require(re.fullmatch(r"0|-?[1-9][0-9]*", value) is not None,
                "non-canonical integer string")
        value = int(value)
    require(type(value) is int and minimum <= value <= maximum, "integer out of range")
    return value


def digest(value, length=64):
    require(isinstance(value, str) and re.fullmatch(rf"[0-9a-f]{{{length}}}", value),
            "invalid digest or revision")
    return value


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def bounded_tree(value, max_depth=128, allow_float=True):
    pending = [(value, 0)]
    seen = set()
    nodes = 0
    while pending:
        item, depth = pending.pop()
        nodes += 1
        require(nodes <= MAX_NODES and depth <= max_depth, "structured value limit exceeded")
        if isinstance(item, (dict, list, tuple)):
            require(id(item) not in seen, "shared/cyclic structured value is not supported")
            seen.add(id(item))
            if isinstance(item, dict):
                for key in item:
                    require(type(key) in (int, str), "invalid map key type")
                children = item.values()
            else:
                children = item
            pending.extend((child, depth + 1) for child in children)
        else:
            require(item is None or type(item) in (str, bytes, int, bool) or
                    (allow_float and type(item) is float and math.isfinite(item)),
                    "non-integral or tagged structured value is not supported")
            if isinstance(item, str):
                require(len(item.encode("utf-8", errors="surrogatepass")) <= MAX_RECORD,
                        "string limit exceeded")


def json_bytes(data):
    require(len(data) <= MAX_RECORD, "JSON record limit exceeded")
    try:
        value = json.loads(data, object_pairs_hook=unique_object,
                           parse_constant=lambda value: (_ for _ in ()).throw(
                               ValueError(f"non-finite JSON value {value}")))
        bounded_tree(value)
    except (UnicodeError, RecursionError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid bounded JSON: {error}") from error
    return value


def safe_relative(name):
    require(isinstance(name, str) and 0 < len(name.encode("utf-8")) <= 4096,
            "invalid relative path length")
    require("\\" not in name and "\x00" not in name and not name.startswith("/"),
            "unsafe relative path")
    parts = name.split("/")
    require(all(part not in ("", ".", "..") and ":" not in part for part in parts),
            "unsafe relative path component")
    return parts


def safe_file(root, name):
    path = Path(root)
    for part in safe_relative(name):
        path = path / part
        require(not path.is_symlink(), "symlink is not an evidence input")
    require(path.is_file(), f"evidence file missing: {name}")
    return path


def file_sha256(path, maximum):
    require(not path.is_symlink(), "artifact symlink is forbidden")
    before = path.stat()
    require(stat.S_ISREG(before.st_mode) and before.st_size <= maximum,
            "file is not regular or exceeds limit")
    digest_value = hashlib.sha256()
    size = 0
    with path.open("rb") as stream:
        while chunk := stream.read(64 * 1024):
            size += len(chunk)
            require(size <= maximum and size <= before.st_size, "file grew while reading")
            digest_value.update(chunk)
    after = path.stat()
    require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) ==
            (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns),
            "file changed while reading")
    require(size == before.st_size, "file length changed while reading")
    return {"bytes": size, "sha256": digest_value.hexdigest()}


def bounded_paths(root):
    """Bound directory entries before sorting, including empty directories."""
    entries = []
    for path in root.rglob("*"):
        require(len(entries) < MAX_FILES, "input directory entry limit exceeded")
        require(not path.is_symlink(), "input symlink is forbidden")
        entries.append(path)
    return entries


def fixture_inventory(root):
    """Inventory every source byte and separately hash UTF-8 after BOM removal.

    decodedUtf8Sha256 describes decoding, not a claim that every README/resource
    was submitted as script. submittedPayloads is checked separately by caller.
    """
    root = Path(root)
    require(root.is_dir() and not root.is_symlink(), "invalid fixture directory")
    entries = []
    names = set()
    total = 0
    for path in sorted(bounded_paths(root)):
        require(not path.is_symlink(), "fixture symlink is forbidden")
        if path.is_dir():
            continue
        name = path.relative_to(root).as_posix()
        safe_relative(name)
        normalized = unicodedata.normalize("NFC", name).lower()
        require(normalized not in names, "fixture path normalization collision")
        names.add(normalized)
        require(len(names) <= MAX_FILES, "fixture file-count limit exceeded")
        raw = file_sha256(path, MAX_SOURCE)
        total += raw["bytes"]
        require(total <= MAX_TRACE, "fixture byte limit exceeded")
        decoded_hash = hashlib.sha256()
        decoder = codecs.getincrementaldecoder("utf-8-sig")("strict")
        utf8 = True
        # A second bounded pass avoids holding a whole resource in memory.
        with path.open("rb") as stream:
            count = 0
            while chunk := stream.read(64 * 1024):
                count += len(chunk)
                require(count <= raw["bytes"], "fixture grew during decoding")
                if utf8:
                    try:
                        decoded_hash.update(decoder.decode(chunk).encode("utf-8"))
                    except UnicodeError:
                        utf8 = False
            if utf8:
                try:
                    decoded_hash.update(decoder.decode(b"", final=True).encode("utf-8"))
                except UnicodeError:
                    utf8 = False
        require(count == raw["bytes"] and file_sha256(path, MAX_SOURCE) == raw,
                "fixture changed during decoding")
        entries.append({"path": name, **raw,
                        "decodedUtf8Sha256": decoded_hash.hexdigest() if utf8 else None})
    return entries


def source_identity(inventory):
    files = [{key: row[key] for key in ("path", "bytes", "sha256")} for row in inventory]
    encoded = json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    return {"files": files, "sha256": hashlib.sha256(encoded).hexdigest()}


def project_payload_hashes(root, inventory):
    """Independent BLAKE3 checks for the actual exported ProjectManifest.

    SHA256 inventory identity remains unchanged; never compare a protocol
    content_hash with a SHA256 source digest or call raw bytes a UTF-8 payload.
    """
    from blake3 import blake3

    result = {}
    for item in inventory:
        path = safe_file(root, item["path"])
        raw, decoded, sha = blake3(), blake3(), hashlib.sha256()
        decoder = codecs.getincrementaldecoder("utf-8-sig")("strict")
        size, decoded_size, utf8 = 0, 0, True
        with path.open("rb") as stream:
            while chunk := stream.read(64 * 1024):
                size += len(chunk)
                require(size <= item["bytes"] and size <= MAX_SOURCE, "project input grew while hashing")
                raw.update(chunk)
                sha.update(chunk)
                if utf8:
                    try:
                        payload = decoder.decode(chunk).encode("utf-8")
                        decoded.update(payload)
                        decoded_size += len(payload)
                    except UnicodeError:
                        utf8 = False
            if utf8:
                try:
                    payload = decoder.decode(b"", final=True).encode("utf-8")
                    decoded.update(payload)
                    decoded_size += len(payload)
                except UnicodeError:
                    utf8 = False
        require(size == item["bytes"] and sha.hexdigest() == item["sha256"],
                "project source changed during BLAKE3 hashing")
        result[item["path"]] = {"rawBlake3": raw.hexdigest(), "rawBytes": size,
                                "decodedUtf8Blake3": decoded.hexdigest() if utf8 else None,
                                "decodedUtf8Bytes": decoded_size if utf8 else None}
    return result


def validate_project_files(files, hashes, required_payloads):
    require(isinstance(files, list) and len(files) <= MAX_FILES, "invalid actual project file inventory")
    seen = set()
    for item in files:
        path = item["relativePath"]
        require(path in hashes and path not in seen, "unknown/duplicate actual project file")
        seen.add(path)
        require(item.get("category") in ("erb", "erh", "csv", "als", "erd", "configuration", "resource"),
                "unknown actual project file category")
        raw = item["category"] == "resource"
        expected = hashes[path]
        hash_key, size_key = ("rawBlake3", "rawBytes") if raw else ("decodedUtf8Blake3", "decodedUtf8Bytes")
        require(item["contentHash"] == expected[hash_key] and expected[hash_key] is not None and
                integer(item["byteLength"]) == expected[size_key], "actual project payload digest/length mismatch")
        require(item["payloadKind"] in (("external", "bytes") if raw else ("utf8",)),
                "actual project payload encoding/category mismatch")
    resources = {path for path in hashes if path.lower().endswith((".png", ".xml", ".txt", ".db", ".sqlite"))}
    require((set(required_payloads) | resources) <= seen,
            "actual project inventory omitted required source/resource")
    return files


def frontend_files(root, kind):
    root = Path(root)
    require(root.is_dir() and not root.is_symlink(), "invalid actual frontend source/bundle root")
    if kind == "frontend_source_manifest":
        files = []
        for directory in ("src", "scripts"):
            base = root / directory
            require(not base.is_symlink(), "frontend source root symlink")
            if base.exists():
                files.extend(bounded_paths(base))
                require(len(files) <= MAX_FILES, "frontend input count limit exceeded")
        for path in root.iterdir():
            name = path.name
            if name in ("package.json", "index.html", "rustyera-core.rev") or any(
                fnmatch.fnmatchcase(name, pattern) for pattern in ("vite.config.*", "tsconfig*.json", ".env*")
            ) or re.search(r"(?:^|[.-])lock(?:[.-]|$)|lock$", name, re.IGNORECASE):
                files.append(path)
                require(len(files) <= MAX_FILES, "frontend input count limit exceeded")
        require(any(path.is_file() and path.is_relative_to(root / "src") for path in files) and
                (root / "package.json").is_file(), "Vite source manifest lacks src/package.json")
    else:
        require(kind == "frontend_file_manifest", "unknown frontend manifest kind")
        files = bounded_paths(root)
    result = []
    for path in sorted(set(files)):
        require(not path.is_symlink(), "frontend source/bundle symlink")
        if path.is_dir():
            continue
        name = path.relative_to(root).as_posix()
        safe_relative(name)
        require(len(result) < MAX_FILES, "frontend input count limit exceeded")
        result.append({"path": name, **file_sha256(path, MAX_STORED)})
    require(result, "empty frontend manifest")
    return result


def validate_frontend_artifact(runtime, path, root):
    require(runtime.get("artifactRole") == "frontend", "frontend artifact role mismatch")
    source = runtime.get("mode") in ("vite-dev", "tauri-test-devserver")
    require(source or runtime.get("mode") in ("embedded", "static-bundle"), "unknown frontend runtime mode")
    expected_kind = "source-manifest" if source else "file-manifest"
    require(runtime.get("artifactKind") == expected_kind and root is not None,
            "frontend artifact kind/root missing")
    manifest = read_manifest(Path(path))
    kind = "frontend_source_manifest" if source else "frontend_file_manifest"
    require(manifest.get("version") == 1 and manifest.get("kind") == kind,
            "wrong frontend source/file manifest")
    actual = frontend_files(root, kind)
    require(manifest.get("files") == actual, "actual frontend source/bundle files differ from manifest")
    return kind, actual


def validate_wasm_assets(identity, runtime_path, root):
    """Reproduce scripts/wasm-assets.ts; this revision is not a core commit."""
    require(root is not None, "actual browser WASM directory is required")
    root = Path(root)
    require(root.is_dir() and not root.is_symlink(), "invalid browser WASM directory")
    names = ("era_web_wasm.js", "era_web_wasm_bg.wasm")
    combined, files = hashlib.sha256(), []
    for name in names:
        path = safe_file(root, name)
        expected = file_sha256(path, MAX_STORED)
        combined.update(name.encode("utf-8"))
        combined.update(b"\0")
        actual, size = hashlib.sha256(), 0
        with path.open("rb") as stream:
            while chunk := stream.read(64 * 1024):
                size += len(chunk)
                require(size <= expected["bytes"], "WASM asset grew during revision hashing")
                combined.update(chunk)
                actual.update(chunk)
        require({"bytes": size, "sha256": actual.hexdigest()} == expected and
                file_sha256(path, MAX_STORED) == expected, "WASM asset changed while hashing")
        files.append({"path": name, **expected})
    result = {"revision": combined.hexdigest(), "files": files}
    require(identity.get("wasmAssets") == result, "actual WASM asset identity mismatch")
    require(Path(runtime_path).resolve() == safe_file(root, names[1]).resolve(),
            "browser runtime artifact is not the actual WASM asset path")
    return result


def read_manifest(path):
    require(path.stat().st_size <= MAX_MANIFEST, "capture manifest limit exceeded")
    with path.open("rb") as stream:
        data = stream.read(MAX_MANIFEST + 1)
    require(len(data) <= MAX_MANIFEST, "capture manifest grew beyond limit")
    return json_bytes(data)


def trace_records(manifest_path, description):
    """Yield bounded records; hashes are verified before completion is emitted."""
    require(description.get("compression") == "gzip", "capture trace must be gzip")
    path = safe_file(manifest_path.parent, description["path"])
    expected = {"bytes": integer(description["storedBytes"], maximum=MAX_STORED),
                "sha256": digest(description["storedSha256"])}
    require(file_sha256(path, MAX_STORED) == expected, "stored trace hash mismatch")
    raw_hash = hashlib.sha256()
    raw_size = 0
    records = 0
    try:
        with gzip.open(path, "rb") as stream:
            while line := stream.readline(MAX_RECORD + 1):
                require(len(line) <= MAX_RECORD and line.endswith(b"\n"),
                        "oversized or incomplete trace record")
                raw_size += len(line)
                records += 1
                require(raw_size <= MAX_TRACE and records <= 500_000,
                        "decoded trace limit exceeded")
                raw_hash.update(line)
                yield json_bytes(line)
    except (OSError, EOFError) as error:
        raise ValueError(f"invalid gzip trace: {error}") from error
    require(raw_size == integer(description["decodedBytes"], maximum=MAX_TRACE) and
            raw_hash.hexdigest() == digest(description["decodedSha256"]),
            "decoded trace hash mismatch")
    require(file_sha256(path, MAX_STORED) == expected, "stored trace changed during parsing")


def decode_payload(value):
    # Mature decoder explicitly rejects duplicate maps, indefinite lengths and
    # depth abuse. Do not replace this with a permissive cbor2.loads fallback.
    import cbor2

    require(isinstance(value, list) and len(value) <= MAX_PAYLOAD, "invalid CBOR byte payload")
    data = bytes(integer(item, maximum=255) for item in value)

    def reject_tag(*_):
        raise ValueError("semantic CBOR tags are not part of this service payload")

    # Override builtin tags too: tag_hook alone sees only unknown tags.
    builtin_tags = (0, 1, 2, 3, 4, 5, 25, 28, 29, 30, 35, 36, 37, 52, 54,
                    100, 256, 258, 260, 261, 55799)
    source = io.BytesIO(data)
    try:
        decoder = cbor2.CBORDecoder(
            source, read_size=1, max_depth=128, allow_indefinite=False,
            allow_duplicate_keys=False, tag_hook=reject_tag,
            semantic_decoders={tag: reject_tag for tag in builtin_tags},
        )
        result = decoder.decode()
    except (TypeError, cbor2.CBORDecodeError) as error:
        raise ValueError("strict CBOR decode failed; cbor2>=6.1.3 is required") from error
    require(source.tell() == len(data), "trailing CBOR payload")
    bounded_tree(result, allow_float=False)
    return result
