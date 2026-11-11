"""Old-worker journal recovery, using real files and deterministic processes."""
import copy
import json
import unittest

import test_instances
import test_host
from host import Machine, canonical, pointer
from test_host import Interrupted


class LegacyRecovery(unittest.TestCase):
    def fixture(self):
        fixture = test_instances.ExecutableReleases()
        fixture.setUp()
        self.addCleanup(fixture.tearDown)
        first = fixture.request()
        fixture.deploy(first)
        fixture.deploy(fixture.request('v2', b'two'))
        request = {**first, 'rollback': True}
        fixture.machine.run(request)
        state = fixture.journal()
        transaction = state.pop('transaction')
        state.pop('rollback')
        edge = fixture.root / 'edge/releases' / state['operation']
        (fixture.root / 'edge/releases' / transaction).rename(edge)
        fixture.machine.record(fixture.root, fixture.root / 'journal.json', state)
        return fixture, request, edge

    def test_prepare_and_direct_activate_recreate_transaction(self):
        for action in ('prepare', 'activate'):
            with self.subTest(action=action):
                fixture, request, edge = self.fixture()
                old = (edge / 'config.json').read_bytes()
                retry = {**request, 'action': action}
                if action == 'activate':
                    for key in ('files', 'backends', 'caddy'):
                        retry.pop(key)
                result = fixture.machine.run(retry)
                self.assertTrue(result['rollback'])
                self.assertNotEqual(result['transaction'], request['operation'])
                self.assertEqual((edge / 'config.json').read_bytes(), old)
                if action == 'prepare':
                    result = fixture.machine.run({**request, 'action': 'activate'})
                self.assertEqual(result['phase'], 'complete')
                self.assertEqual(set(fixture.runtime.processes.running), {fixture.release('v1')})

    def test_request_mismatch_leaves_legacy_candidate_untouched(self):
        fixture, request, _ = self.fixture()
        changed_host = copy.deepcopy(request)
        changed_host['host']['executables']['services.compiler']['account'] = 'other'
        before = (fixture.root / 'journal.json').read_bytes()
        events = list(fixture.runtime.processes.events)
        for changed in (changed_host, {**request, 'version': 'other'}, {**request, 'operation': 'a' * 64}):
            with self.subTest(request=changed), self.assertRaises(ValueError):
                fixture.machine.run(changed)
            self.assertEqual((fixture.root / 'journal.json').read_bytes(), before)
            self.assertEqual(fixture.runtime.processes.events, events)

    def test_prior_and_candidate_substitution_cannot_enter_cleanup(self):
        for target in ('prior_edge', 'prior_backend', 'candidate'):
            with self.subTest(target=target):
                fixture, request, _ = self.fixture()
                state = fixture.journal()
                paths = {'prior_edge': fixture.root / 'edge' / state['previous_edge'] / 'config.json',
                         'prior_backend': fixture.release('v2') / 'backend',
                         'candidate': fixture.release('v1') / 'backend'}
                paths[target].write_bytes(b'changed')
                before = (fixture.root / 'journal.json').read_bytes()
                events = list(fixture.runtime.processes.events)
                with self.assertRaises(ValueError):
                    fixture.machine.run(request)
                self.assertEqual((fixture.root / 'journal.json').read_bytes(), before)
                self.assertEqual(fixture.runtime.processes.events, events)

    def test_interrupted_legacy_recovery_retains_new_intent(self):
        for point in ('legacy_recovery', 'legacy_restored'):
            for action in ('prepare', 'activate'):
                with self.subTest(point=point, action=action):
                    fixture, request, _ = self.fixture()
                    def fault(at):
                        if at == point:
                            raise Interrupted()
                    with self.assertRaises(Interrupted):
                        Machine(fixture.runtime, fault).run({**request, 'action': action})
                    before = (fixture.root / 'journal.json').read_bytes()
                    events = list(fixture.runtime.processes.events)
                    with self.assertRaisesRegex(ValueError, 'rollback intent differs'):
                        fixture.machine.run({**request, 'rollback': False})
                    self.assertEqual((fixture.root / 'journal.json').read_bytes(), before)
                    self.assertEqual(fixture.runtime.processes.events, events)
                    self.assertEqual(fixture.deploy(request)['phase'], 'complete')

    def test_recovery_rebuilds_stale_unselected_route(self):
        fixture, request, edge = self.fixture()
        # An older worker could reuse a historical edge while its journal
        # correctly pinned the newer active edge. Never promote that edge.
        state = fixture.journal()
        previous = fixture.root / 'edge' / state['previous_edge'] / 'config.json'
        template = request['host']['edge']
        template['routes'].append({'handler': 'reverse_proxy', 'upstreams': [{'dial': '127.0.0.1:9002'}]})
        for path, port in ((previous, 49002), (edge / 'config.json', 49000)):
            config = json.loads(path.read_bytes())
            config['routes'].append({'handler': 'reverse_proxy', 'upstreams': [{'dial': f'127.0.0.1:{port}'}]})
            path.write_bytes(canonical(config))
            (path.parent / 'template.json').write_bytes(canonical(template))
        # Model the independently pinned old worker's journal, not a
        # replacement of inputs after its pins have been recorded.
        from host import digest
        state['binding'] = digest(canonical(request['host']))
        state['rendered_edge_sha256'] = digest((edge / 'config.json').read_bytes())
        for name in state['previous_edge_pins']:
            state['previous_edge_pins'][name] = digest((previous.parent / name).read_bytes())
        fixture.machine.record(fixture.root, fixture.root / 'journal.json', state)
        result = fixture.deploy(request)
        config = json.loads((fixture.root / 'edge/current/config.json').read_bytes())
        self.assertEqual(result['phase'], 'complete')
        self.assertEqual(config['routes'][1]['upstreams'][0]['dial'], '127.0.0.1:49002')
        self.assertNotEqual(pointer(fixture.root / 'edge/current'), 'releases/' + request['operation'])


class OriginalStaticRecovery(unittest.TestCase):
    """Original static journal shape, also checked with the real historical worker."""
    def fixture(self, proxy=False):
        fixture = test_host.Releases()
        fixture.setUp()
        self.addCleanup(fixture.tearDown)
        request = fixture.request()
        if proxy:
            request['host']['edge'] = {'routes': [{'handler': 'reverse_proxy', 'upstreams': [{'dial': '127.0.0.1:9001'}]}]}
        fixture.machine.run(request)
        edge = self.original_journal(fixture, request)

        return fixture, request, edge

    @staticmethod
    def original_journal(fixture, request):
        journal = fixture.root / 'journal.json'
        state = json.loads(journal.read_bytes())
        transaction = state.pop('transaction')
        for field in ('rollback', 'rendered_edge_sha256', 'installed_processes',
                      'previous_processes', 'observations'):
            state.pop(field)
        edge = fixture.root / 'edge/releases' / request['operation']
        (fixture.root / 'edge/releases' / transaction).rename(edge)
        (edge / 'template.json').unlink()
        fixture.machine.record(fixture.root, journal, state)

        return edge

    def test_original_static_prepare_and_payload_free_activation(self):
        for action in ('prepare', 'activate'):
            with self.subTest(action=action):
                f, request, _ = self.fixture()
                retry = {**request, 'action': action, 'recovery': True}
                if action == 'activate':
                    retry.pop('files')
                    retry.pop('caddy')
                result = f.machine.run(retry)
                if action == 'prepare':
                    result = f.machine.run({**retry, 'action': 'activate'})
                self.assertEqual(result['phase'], 'complete')
                self.assertNotEqual(result['transaction'], request['operation'])
                self.assertEqual(f.current(), 'releases/v1')

    def test_static_inputs_are_checked_before_recovery_mutation(self):
        for target in ('assets', 'caddy', 'config', 'mode'):
            with self.subTest(target=target):
                f, request, edge = self.fixture()
                if target == 'assets':
                    (f.root / 'units/frontends.docs/releases/v1/assets/index.html').write_bytes(b'changed')
                elif target == 'caddy':
                    (edge / 'caddy').write_bytes(b'changed')
                elif target == 'config':
                    (edge / 'config.json').write_bytes(b'{}')
                else:
                    (edge / 'config.json').chmod(0o600)
                before = (f.root / 'journal.json').read_bytes()
                restarts = f.runtime.restarts
                with self.assertRaises(ValueError):
                    f.machine.run({**request, 'recovery': True})
                self.assertEqual((f.root / 'journal.json').read_bytes(), before)
                self.assertEqual(f.runtime.restarts, restarts)
                self.assertIsNone(f.current())

    def test_partial_derived_evidence_never_downgrades_to_static(self):
        for field in ('installed_processes', 'previous_processes'):
            with self.subTest(field=field):
                f, request, _ = self.fixture()
                journal = f.root / 'journal.json'
                state = json.loads(journal.read_bytes())
                state[field] = {}
                f.machine.record(f.root, journal, state)
                before = journal.read_bytes()
                with self.assertRaisesRegex(ValueError, 'rendered edge changed'):
                    f.machine.run({**request, 'recovery': True})
                self.assertEqual(journal.read_bytes(), before)

    def test_executable_journal_cannot_drop_all_derived_checks(self):
        fixture = test_instances.ExecutableReleases()
        fixture.setUp()
        self.addCleanup(fixture.tearDown)
        request = fixture.request()
        fixture.machine.instances.allocate = lambda excluded: '127.0.0.1:9001'
        fixture.machine.run(request)
        self.original_journal(fixture, request)
        journal = fixture.root / 'journal.json'
        before = journal.read_bytes()
        events = list(fixture.runtime.processes.events)
        with self.assertRaisesRegex(ValueError, 'cannot authorize executable recovery'):
            fixture.machine.run({**request, 'recovery': True})
        self.assertEqual(journal.read_bytes(), before)
        self.assertEqual(fixture.runtime.processes.events, events)

    def test_static_configuration_cannot_acquire_derived_route_bindings(self):
        fixture, request, edge = self.fixture(proxy=True)
        (edge / 'template.json').write_bytes(canonical(request['host']['edge']))
        changed = copy.deepcopy(request['host']['edge'])
        changed['routes'][0]['upstreams'][0]['dial'] = '127.0.0.1:9002'
        (edge / 'config.json').write_bytes(canonical(changed))
        before = (fixture.root / 'journal.json').read_bytes()
        with self.assertRaisesRegex(ValueError, 'static configuration differs'):
            fixture.machine.run({**request, 'recovery': True})
        self.assertEqual((fixture.root / 'journal.json').read_bytes(), before)
        self.assertIsNone(fixture.current())


if __name__ == '__main__':
    unittest.main()
