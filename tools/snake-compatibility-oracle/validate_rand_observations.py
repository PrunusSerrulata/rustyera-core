#!/usr/bin/env python3
"""Accept one RAND observation without erasing warning projection differences."""
import argparse
import hashlib
import json
import re
from pathlib import Path

from comparison import same_json_value, validate_rust_evidence, validate_upstream_load_diagnostics



def parse_rand_message(line, *, clamped):
    # Decode the fixed English resource wire format into entry and integer value.
    # Preserve raw output separately; localized text is not a shared diagnostic code.
    suffixes = {"variable": "（已钳制为 0，不再中断运行）", "function": "（已钳制为下界，不再中断运行）"}
    patterns = {"variable": r"RAND was specified with a value less than or equal to 0 \((-?[0-9]+)\)",
                "function": r"RAND: Specified with a maximum value equal to or less than the minimum value \((-?[0-9]+)\)"}
    if not isinstance(line, str):
        return None
    for entry, pattern in patterns.items():
        match = re.fullmatch(pattern + (re.escape(suffixes[entry]) if clamped else ""), line)
        if match:
            return {"entry": entry, "maximum": int(match[1])}
    return None


def validate(evidence, rust_evidence, manifest):
    if evidence.get('status') != 'completed_observations':
        raise ValueError('incomplete capture')
    compared = evidence.get('rustComparison', {}).get('cases', [])
    if len(compared) != 1:
        raise ValueError('accept exactly one case')
    actual = compared[0]
    case = next(c for c in manifest['cases'] if c['id'] == actual['case'])
    oracle = evidence['oracle']
    if case['group'] != 'RNG' or oracle not in case['allowedOracles']:
        raise ValueError('wrong fixture or oracle')
    if evidence['semanticBaseline'] != manifest['semanticBaselines'][oracle]:
        raise ValueError('wrong semantic baseline')
    validate_rust_evidence(rust_evidence, oracle, evidence['sourceFixture'], manifest['seed'],
                           manifest['requiredRustPolicy'][oracle])
    if not same_json_value(evidence['rust']['profile'], rust_evidence['profile']):
        raise ValueError('Rust identities differ')
    rust_case = next(c for c in rust_evidence['cases'] if c['id'] == case['id'])
    if rust_case.get('load', {}).get('success') is not True:
        raise ValueError('Rust project load did not succeed')
    load = validate_upstream_load_diagnostics(rust_case, actual['oracleLoad'], rust_evidence['profile'], duplicate_alias=False)
    if load['differences']:
        raise ValueError('unexpected load diagnostics')
    if len(actual['steps']) != 1:
        raise ValueError('expected one operation')
    step = actual['steps'][0]
    rust = step['rust']['result']
    reference = step['oracle']['result']
    planned = case['requests'][0]['expect'][oracle]['result']
    for observed in (rust, reference):
        if not same_json_value(observed.get('watches'), planned['watches']):
            raise ValueError('unexpected watches')
    is_error = planned['termination'] == 'error'
    required = {'ok', 'watches', 'executionOutcome'} if is_error else {'ok', 'termination', 'output', 'watches'}
    if not required.issubset(set(step.get('compared', []))):
        raise ValueError('missing mandatory comparison fields')
    if (step['oracle'].get('ok') is not True or rust.get('ok') is not (not is_error)
            or rust.get('termination') != ('faulted' if is_error else 'completed')
            or reference.get('termination') != planned['termination']):
        raise ValueError('unexpected request success or terminal state')
    warning = case.get('registeredRandWarningProjection')
    if warning:
        if oracle != 'snake' or rust.get('termination') != 'completed' or reference.get('termination') != 'completed':
            raise ValueError('clamped operation did not complete')
        diagnostics = step['diagnosticComparison']
        notes = diagnostics['rust']
        if diagnostics['oracle'] != [] or diagnostics.get('oracleError') is not None:
            raise ValueError('unexpected oracle diagnostic')
        if not isinstance(notes, list) or [n.get('code') for n in notes] != warning['rustCodes']:
            raise ValueError('wrong warning entries/count/order')
        for note in notes:
            context = note.get('context') or {}
            source = note.get('source') or {}
            if (note.get('level') != 'warning' or note.get('notification') != 'log_only'
                    or context.get('stage') != 'runtime'
                    or not same_json_value(context.get('identity'), rust_evidence['profile'])
                    or source.get('relative_path') != 'erb/rng.erb'
                    or type(source.get('line')) is not int
                    or not isinstance(context.get('api'), str)
                    or not context['api']):
                raise ValueError('warning lacks runtime origin/identity')
        differences = step['differences']
        if len(differences) != 1 or differences[0]['field'] != 'output':
            raise ValueError('difference exceeds warning projection')
        output = differences[0]
        if (output['rust'] != [] or not isinstance(output['oracle'], list)
                or len(output['oracle']) != warning['oracleOutputLines']
                or not all(isinstance(line, str) and line for line in output['oracle'])
                or step['status'] != 'different' or actual['status'] != 'different'):
            raise ValueError('unexpected warning projection shape')
        parsed = [parse_rand_message(line, clamped=True) for line in output['oracle']]
        if not same_json_value(parsed, warning['oracleWarnings']):
            raise ValueError('wrong RAND warning entry or parameter')
        status = 'accepted_registered_warning_projection'
    elif planned['termination'] == 'error':
        rejection = step.get('rejectionComparison') or {}
        if (step['differences'] or rejection.get('status') != 'matched_observed_rejection'
                or rust.get('termination') != 'faulted' or reference.get('termination') != 'error'):
            raise ValueError('original rejection changed')
        notes = step['diagnosticComparison']['rust']
        if not isinstance(notes, list) or len(notes) != 1:
            raise ValueError('expected one original native fault')
        fault = notes[0]
        primary = (fault.get('vm') or {}).get('primary') or {}
        source = (fault.get('origin') or {}).get('source') or {}
        if (fault.get('code') != 'vm_fault' or primary.get('category') != 'script_argument'
                or primary.get('code') != 'native' or source.get('relative_path') != 'erb/rng.erb'):
            raise ValueError('original error is not the RAND argument rejection')
        lines = reference.get('output')
        if not isinstance(lines, list) or not all(isinstance(line, str) for line in lines):
            raise ValueError('missing original error output')
        errors = [line.removeprefix('Error description: ') for line in lines
                  if line.startswith('Error description: ')]
        parsed = [parse_rand_message(line, clamped=False) for line in errors]
        if not same_json_value(parsed, [case['expectedOracleRandError']]):
            raise ValueError('original oracle error is not the expected RAND argument rejection')
        status = 'matched_observed_rejection_diagnostics_incomparable'
    else:
        if actual['status'] != 'matched_observables' or step['status'] != 'matched_observables':
            raise ValueError('unexpected valid RAND difference')
        status = 'matched_observables'
    return {'case': case['id'], 'oracle': oracle, 'status': status, 'rawVerdict': actual['status'],
            'registeredDifference': warning, 'loadDiagnosticStatus': load['status']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('fixture', 'evidence', 'rust', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    raw = args.evidence.read_bytes()
    result = validate(json.loads(raw), json.loads(args.rust.read_bytes()), json.loads((args.fixture / 'cases.json').read_bytes()))
    result['evidenceSha256'] = hashlib.sha256(raw).hexdigest()
    result['validatorSha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(result, output, indent=2)
        output.write('\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
