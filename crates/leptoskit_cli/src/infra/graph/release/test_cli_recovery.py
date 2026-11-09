"""Historical CLI request identities at the real worker boundary.

Rust plan normalization and provider checks have their own tests. This suite
uses deterministic adapters and does not claim CLI/systemd/SSH execution.
"""
import copy
import unittest

import test_host
from host import canonical, digest


class HistoricalContext(unittest.TestCase):
    def setUp(self):
        self.fixture = test_host.Releases()
        self.fixture.setUp()
        self.addCleanup(self.fixture.tearDown)

    def context(self):
        request = self.fixture.request()
        plan = {'format_version': 1, 'kind': 'graph-static-release', 'version': 'v1',
                'installer_sha256': 'a' * 64, 'transport_sha256': 'b' * 64,
                'hosts': {'web': copy.deepcopy(request['host'])}}
        request['operation'] = digest(canonical(plan))

        return request, plan

    def test_replanned_context_cannot_claim_original_operation(self):

        request, old = self.context()
        self.fixture.machine.run(request)
        before = (self.fixture.root / 'journal.json').read_bytes()
        current = copy.deepcopy(old)
        current['kind'] = 'graph-release'
        current['installer_sha256'] = 'c' * 64
        host = current['hosts']['web']
        host.update(executables={}, linux_formats={})

        for operation in (digest(canonical(current)), request['operation']):
            changed = {**request, 'host': host, 'operation': operation, 'recovery': True}
            with self.assertRaises(ValueError):
                self.fixture.machine.run(changed)
            self.assertEqual((self.fixture.root / 'journal.json').read_bytes(), before)
        recovered = {**request, 'recovery': True}
        self.fixture.machine.run(recovered)
        result = self.fixture.machine.run({**recovered, 'action': 'activate'})
        self.assertEqual(result['operation'], digest(canonical(old)))
        self.assertEqual(result['phase'], 'complete')

    def test_recovery_never_starts_an_absent_operation(self):
        request, _ = self.context()
        with self.assertRaisesRegex(ValueError, 'original journal operation'):
            self.fixture.machine.run({**request, 'recovery': True})
        self.assertFalse((self.fixture.root / 'journal.json').exists())
        self.assertFalse((self.fixture.root / 'units').exists())

    def test_recovery_cannot_resurrect_an_older_completed_operation(self):
        request, _ = self.context()
        self.fixture.deploy(request)
        self.fixture.deploy(self.fixture.request('v2', b'next'))
        before = (self.fixture.root / 'journal.json').read_bytes()
        with self.assertRaisesRegex(ValueError, 'original journal operation'):
            self.fixture.machine.run({**request, 'recovery': True})
        self.assertEqual((self.fixture.root / 'journal.json').read_bytes(), before)
        self.assertEqual(self.fixture.current(), 'releases/v2')

    def test_recovery_status_observes_rollback_without_changing_intent(self):

        request, _ = self.context()
        self.fixture.deploy(request)
        self.fixture.deploy(self.fixture.request('v2', b'next'))
        rollback = {**request, 'rollback': True}
        self.fixture.machine.run(rollback)
        before = (self.fixture.root / 'journal.json').read_bytes()
        result = self.fixture.machine.run({**request, 'action': 'status', 'recovery': True})
        self.assertTrue(result['journal']['rollback'])
        self.assertEqual((self.fixture.root / 'journal.json').read_bytes(), before)
        with self.assertRaisesRegex(ValueError, 'rollback intent differs'):
            self.fixture.machine.run({**request, 'recovery': True})
        self.assertEqual((self.fixture.root / 'journal.json').read_bytes(), before)

    def test_recovery_requires_boolean_and_original_version(self):
        request, _ = self.context()
        self.fixture.machine.run(request)
        before = (self.fixture.root / 'journal.json').read_bytes()

        for change in ({'recovery': 'yes'}, {'recovery': True, 'version': 'other'}):
            with self.assertRaises(ValueError):
                self.fixture.machine.run({**request, **change})
            self.assertEqual((self.fixture.root / 'journal.json').read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
