"""Pure watchdog comparison tests; no oracle process is started."""

import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("oracle_smoke_supervisor", Path(__file__).with_name("oracle-smoke-supervisor.py"))
supervisor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(supervisor)


class StateComparisonTests(unittest.TestCase):
    def test_transport_id_and_response_counter_are_not_state_progress(self):
        first = {"case": 1, "responsesCompleted": 1, "pending": {"id": 1, "op": "run"}, "lastFullResponse": {"id": 1, "result": {"watches": {"id": 7}}}}
        second = {"case": 2, "responsesCompleted": 2, "pending": {"id": 2, "op": "run"}, "lastFullResponse": {"id": 2, "result": {"watches": {"id": 7}}}}
        self.assertTrue(supervisor.unchanged(first, second))
        self.assertEqual(second["pending"]["id"], 2)
        second["lastFullResponse"]["result"]["watches"]["id"] = 8
        self.assertFalse(supervisor.unchanged(first, second))

    def test_stall_uses_complete_response(self):
        state = {"pending": {"op": "run"}, "lastFullResponse": {"watches": {"FLAG:0": 1}}, "process": {"returncode": None}}
        self.assertFalse(supervisor.unchanged(None, state))
        self.assertTrue(supervisor.unchanged(state, dict(state)))
        changed = {**state, "lastFullResponse": {"watches": {"FLAG:0": 2}}}
        self.assertFalse(supervisor.unchanged(state, changed))


if __name__ == "__main__":
    unittest.main()
