#!/usr/bin/env python3
"""Verify captured tuples and installed maps offline, without running the provider."""

import argparse
import hashlib
import json
import struct
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--maps', type=Path, required=True)
    parser.add_argument('--provider', type=Path, required=True)
    parser.add_argument('--generator', type=Path, required=True)
    args = parser.parse_args()
    manifest_path = args.capture / 'manifest.json'
    manifest = json.loads(manifest_path.read_bytes())
    provenance = json.loads((args.maps / 'provenance.json').read_bytes())
    assert digest(manifest_path) == (args.capture / 'manifest.sha256').read_text().split()[0]
    assert manifest['violations'] == []
    assert manifest['codePages'] == [932, 949, 936, 950]
    for key, value in provenance.items():
        actual = manifest[key]
        if key == 'provider':
            actual = {k: v for k, v in actual.items() if k != 'path'}
        assert value == actual, key
    assert digest(args.provider) == manifest['provider']['sha256']
    assert digest(args.generator) == manifest['generatorAssembly']['sha256']
    for name, expected in manifest['generatorSources'].items():
        assert digest(Path(__file__).parent / name) == expected, name
    for name, expected in manifest['outputs'].items():
        path = args.capture / name
        assert path.stat().st_size == expected['bytes'], name
        assert digest(path) == expected['sha256'], name
    rows = {}
    for cp in manifest['codePages']:
        raw = (args.capture / f'cp{cp}.tuples.bin').read_bytes()
        assert len(raw) == 65536 * 7
        rows[cp] = list(struct.iter_unpack('<HIB', raw))
        for index, (unit, width, exact) in enumerate(rows[cp]):
            assert unit == index and width in (1, 2) and exact in (0, 1), (cp, index)
    for cp, values in rows.items():
        bits = bytearray(8192)
        for unit, width, exact in values:
            _, sjis_width, sjis_exact = rows[932][unit]
            final = width if exact or not sjis_exact else sjis_width
            if unit < 128:
                assert final == 1, (cp, unit, 'ASCII fast path')
            bits[unit >> 3] |= (final - 1) << (unit & 7)
        name = f'cp{cp}.final-width.bits'
        assert bits == (args.capture / name).read_bytes(), name
        assert bits == (args.maps / name).read_bytes(), name
    print('Verified four maps, 262144 tuples, provenance, provider, generator and ASCII widths')


if __name__ == '__main__':
    main()
