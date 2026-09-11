import copy
import unittest

from validate_rand_observations import validate


class RandObservations(unittest.TestCase):
    def sample(self, *, original=False):
        oracle = 'original' if original else 'snake'
        version = 3 if original else 14
        identity = {'profile': 'emuera.em' if original else 'emuera.skia.snake',
                    'semantic_version': version, 'policy_version': version,
                    'rng_algorithm': 'sfmt19937', 'rng_state_version': 1, 'layout': 'unicode_column_v1',
                    'arithmetic': 'wrapping_i64_v1' if original else 'snake_saturating_i64_v1',
                    'save_codec': 'emuera1808' if original else 'snake_emuera1808_interop_v1',
                    'services': [] if original else [{'name': n, 'version': 1} for n in
                                 ('rustyera.sql', 'rustyera.sql.limits', 'rustyera.scene', 'rustyera.audio')]}
        result = {'termination': 'error' if original else 'completed', 'watches': {'RESULT:10': 0}}
        case = {'id': 'rand', 'group': 'RNG', 'allowedOracles': [oracle],
                'requests': [{'expect': {oracle: {'result': result}}}]}
        operation = {'ok': not original, 'termination': 'faulted' if original else 'completed',
                     'watches': {'RESULT:10': 0}}
        reference = copy.deepcopy(result)
        if original:
            case['expectedOracleRandError'] = {'entry': 'function', 'maximum': 7}
            reference['output'] = ['Error description: RAND: Specified with a maximum value equal to or less than the minimum value (7)']
            notes = [{'code': 'vm_fault', 'origin': {'source': {'relative_path': 'erb/rng.erb'}},
                      'vm': {'primary': {'category': 'script_argument', 'code': 'native'}}}]
            status = 'incomparable'
            compared = ['ok', 'executionOutcome', 'watches']
            differences = []
        else:
            case['registeredRandWarningProjection'] = {
                'rustCodes': ['compat.rand.variable_range', 'compat.rand.function_range'],
                'oracleOutputLines': 2,
                'oracleWarnings': [{'entry': 'variable', 'maximum': -1}, {'entry': 'function', 'maximum': 7}]}
            notes = [{'code': code, 'level': 'warning', 'notification': 'log_only',
                      'source': {'relative_path': 'erb/rng.erb', 'line': i},
                      'context': {'identity': identity, 'stage': 'runtime', 'api': 'ASSIGN'}}
                     for i, code in enumerate(case['registeredRandWarningProjection']['rustCodes'])]
            differences = [{'field': 'output', 'rust': [], 'oracle': [
                'RAND was specified with a value less than or equal to 0 (-1)（已钳制为 0，不再中断运行）',
                'RAND: Specified with a maximum value equal to or less than the minimum value (7)（已钳制为下界，不再中断运行）']}]
            status = 'different'
            compared = ['ok', 'termination', 'watches', 'output']
        step = {'rust': {'result': operation}, 'oracle': {'ok': True, 'result': reference},
                'diagnosticComparison': {'rust': notes, 'oracle': [], 'oracleError': None},
                'differences': differences, 'compared': compared, 'status': status,
                'rejectionComparison': {'status': 'matched_observed_rejection'} if original else None}
        actual = {'case': 'rand', 'status': status, 'steps': [step],
                  'oracleLoad': {'diagnostics': [], 'result': {'output': ['Now Loading...']}}}
        fixture = {'files': []}
        evidence = {'status': 'completed_observations', 'oracle': oracle, 'semanticBaseline': 'fixed',
                    'sourceFixture': fixture, 'rust': {'profile': identity}, 'rustComparison': {'cases': [actual]}}
        rust = {'version': 1, 'seed': 1, 'coreSha': 'a' * 40, 'dirty': True, 'sourceFixture': fixture,
                'profile': identity, 'cases': [{'id': 'rand', 'load': {'success': True, 'diagnostics': []}}]}
        manifest = {'seed': 1, 'cases': [case], 'semanticBaselines': {oracle: 'fixed'},
                    'requiredRustPolicy': {oracle: {'semantic_version': version, 'policy_version': version}}}
        return evidence, rust, manifest

    def test_warning_projection_retains_raw_difference(self):
        result = validate(*self.sample())
        self.assertEqual(result['rawVerdict'], 'different')
        self.assertEqual(result['status'], 'accepted_registered_warning_projection')
        self.assertEqual(validate(*self.sample(original=True))['status'],
                         'matched_observed_rejection_diagnostics_incomparable')

    def test_valid_branch_requires_completed_state_even_when_verdict_matches(self):
        evidence, rust, manifest = self.sample()
        del manifest['cases'][0]['registeredRandWarningProjection']
        actual = evidence['rustComparison']['cases'][0]
        actual['status'] = 'matched_observables'
        step = actual['steps'][0]
        step['status'] = 'matched_observables'
        step['differences'] = []
        step['diagnosticComparison']['rust'] = []
        self.assertEqual(validate(evidence, rust, manifest)['status'], 'matched_observables')
        step['rust']['result']['termination'] = 'waiting_input'
        step['oracle']['result']['termination'] = 'waiting_input'
        with self.assertRaises(ValueError):
            validate(evidence, rust, manifest)

    def test_extra_warning_wrong_origin_and_context_are_rejected(self):
        for mutation in ('extra', 'source', 'identity', 'notification'):
            evidence, rust, manifest = self.sample()
            notes = evidence['rustComparison']['cases'][0]['steps'][0]['diagnosticComparison']['rust']
            if mutation == 'extra':
                notes.append(copy.deepcopy(notes[0]))
            elif mutation == 'source':
                notes[0]['source']['relative_path'] = 'unrelated.erb'
            elif mutation == 'identity':
                notes[0]['context']['identity'] = {'profile': 'emuera.em'}
            else:
                notes[0]['notification'] = 'modal'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate(evidence, rust, manifest)

    def test_wrong_warning_entry_parameter_and_extra_output_are_rejected(self):
        for mutation in ('text', 'entry', 'parameter', 'line', 'difference', 'watch'):
            evidence, rust, manifest = self.sample()
            step = evidence['rustComparison']['cases'][0]['steps'][0]
            lines = step['differences'][0]['oracle']
            if mutation == 'text':
                lines[0] = 'unrelated error'
            elif mutation == 'entry':
                lines[0] = lines[1]
            elif mutation == 'parameter':
                lines[0] = lines[0].replace('(-1)', '(-2)')
            elif mutation == 'line':
                lines.append('extra output')
            elif mutation == 'difference':
                step['differences'].append({'field': 'termination'})
            else:
                step['rust']['result']['watches'] = {'RESULT:10': 1}
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                validate(evidence, rust, manifest)

    def test_terminal_state_success_and_required_fields_are_checked(self):
        for original in (False, True):
            for mutation in ('termination', 'request', 'field'):
                evidence, rust, manifest = self.sample(original=original)
                step = evidence['rustComparison']['cases'][0]['steps'][0]
                if mutation == 'termination':
                    step['rust']['result']['termination'] = 'waiting_input'
                    step['oracle']['result']['termination'] = 'waiting_input'
                elif mutation == 'request':
                    step['oracle']['ok'] = False
                else:
                    step['compared'].remove('watches')
                with self.subTest(original=original, mutation=mutation), self.assertRaises(ValueError):
                    validate(evidence, rust, manifest)

    def test_missing_error_envelope_and_failed_load_are_rejected(self):
        evidence, rust, manifest = self.sample(original=True)
        lines = evidence['rustComparison']['cases'][0]['steps'][0]['oracle']['result']['output']
        lines[0] = lines[0].removeprefix('Error description: ')
        with self.assertRaises(ValueError):
            validate(evidence, rust, manifest)
        evidence, rust, manifest = self.sample()
        rust['cases'][0]['load']['success'] = False
        with self.assertRaises(ValueError):
            validate(evidence, rust, manifest)

    def test_another_oracle_error_is_not_a_rand_rejection(self):
        evidence, rust, manifest = self.sample(original=True)
        evidence['rustComparison']['cases'][0]['steps'][0]['oracle']['result']['output'] = ['Array index out of range']
        with self.assertRaises(ValueError):
            validate(evidence, rust, manifest)


if __name__ == '__main__':
    unittest.main()
