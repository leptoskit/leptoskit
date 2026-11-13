"""Benign deterministic review regressions; no real services are launched."""
import copy
import json
import unittest

import test_instances
from host import canonical, digest


class ReviewRegressions(unittest.TestCase):
    def setUp(self):
        self.fixture = test_instances.ExecutableReleases()
        self.fixture.setUp()
        self.addCleanup(self.fixture.tearDown)
        ports = iter(range(49000, 49100))
        self.fixture.machine.instances.allocate = lambda excluded: f'127.0.0.1:{next(ports)}'

    def test_rollback_preserves_unselected_effective_route(self):
        fixture = self.fixture
        template = {'routes': [
            {'handler': 'reverse_proxy', 'upstreams': [{'dial': '127.0.0.1:9001'}]},
            {'handler': 'reverse_proxy', 'upstreams': [{'dial': '127.0.0.1:9002'}]},
        ]}
        def request(version, unit):
            request = fixture.request(version, version.encode(), unit)
            request['host']['edge'] = copy.deepcopy(template)

            if unit == 'services.accounts':
                contract = request['host']['executables'][unit]
                contract['address'] = contract['environment']['PLAY_BIND'] = '127.0.0.1:9002'
            request['operation'] = digest(canonical([version, request['host']]))

            return request
        fixture.deploy(request('b1', 'services.accounts'))

        a1 = request('a1', 'services.compiler')
        first = fixture.deploy(a1)
        original_edge = fixture.root / 'edge/releases' / first['transaction'] / 'config.json'
        original_routes = original_edge.read_bytes()
        original_history = (fixture.root / 'transactions' / (first['transaction'] + '.json')).read_bytes()
        fixture.deploy(request('b2', 'services.accounts'))
        fixture.deploy(request('a2', 'services.compiler'))
        expected = fixture.runtime.processes.running[fixture.release('b2', 'services.accounts')]['address']
        result = fixture.deploy({**a1, 'rollback': True})
        actual = json.loads((fixture.root / 'edge/current/config.json').read_bytes())['routes'][1]['upstreams'][0]['dial']
        self.assertEqual(result['phase'], 'complete')
        self.assertNotEqual(first['transaction'], result['transaction'])
        self.assertEqual(original_edge.read_bytes(), original_routes)
        self.assertEqual((fixture.root / 'transactions' / (first['transaction'] + '.json')).read_bytes(), original_history)
        self.assertEqual(actual, expected, 'completed rollback restored a stopped unselected instance route')

    def test_version_mismatch_does_not_cancel_prepared_candidate(self):
        fixture = self.fixture
        request = fixture.request()
        fixture.machine.run(request)
        journal = (fixture.root / 'journal.json').read_bytes()
        events = list(fixture.runtime.processes.events)
        with self.assertRaises(ValueError):
            fixture.machine.run({**request, 'version': 'different'})
        self.assertEqual(fixture.journal()['phase'], 'prepared', 'rejected context canceled the original operation')

        self.assertEqual((fixture.root / 'journal.json').read_bytes(), journal)
        self.assertEqual(events, fixture.runtime.processes.events)
        self.assertFalse(fixture.release('different').exists())
        self.assertIn(fixture.release('v1'), fixture.runtime.processes.running)

    def test_rejected_host_intent_and_action_preserve_prepared_context(self):
        fixture = self.fixture
        request = fixture.request()
        fixture.machine.run(request)
        journal = (fixture.root / 'journal.json').read_bytes()
        events = list(fixture.runtime.processes.events)
        changed_host = copy.deepcopy(request)
        changed_host['host']['executables']['services.compiler']['environment']['FEATURE'] = 'changed'
        rejected = [changed_host, {**request, 'rollback': True}, {**request, 'rollback': 'true'},
                    {**request, 'action': 'unknown'}, {**request, 'action': 'activate', 'version': 'different'}]

        for candidate in rejected:
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                fixture.machine.run(candidate)
            self.assertEqual((fixture.root / 'journal.json').read_bytes(), journal)
            self.assertEqual(events, fixture.runtime.processes.events)
        result = fixture.machine.run({**request, 'action': 'activate'})
        self.assertEqual(result['phase'], 'complete')

    def test_prior_edge_change_after_preparation_prevents_promotion(self):
        fixture = self.fixture
        first = fixture.deploy(fixture.request())
        second = fixture.request('v2', b'second')
        fixture.machine.run(second)
        current = fixture.root / 'edge/releases' / first['transaction'] / 'config.json'
        value = json.loads(current.read_bytes())
        value['routes'][0]['upstreams'][0]['dial'] = '127.0.0.1:49199'
        current.write_bytes(canonical(value))

        with self.assertRaisesRegex(ValueError, 'previous edge changed'):
            fixture.machine.run({**second, 'action': 'activate'})
        self.assertNotEqual(fixture.journal()['phase'], 'complete')
        self.assertIn(fixture.release('v1'), fixture.runtime.processes.running)
        self.assertNotIn(fixture.release('v2'), fixture.runtime.processes.running)


if __name__ == '__main__':
    unittest.main()
