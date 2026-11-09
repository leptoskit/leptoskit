"""Private graph hosts use the real worker with deterministic process adapters.

These tests do not establish real Linux/systemd or cross-host network behavior.
"""
import copy
import json
import pathlib
import types
import unittest
from unittest.mock import patch

from host import Linux, Machine, canonical, digest, pointer
from test_host import Interrupted
import test_instances
from test_instances import ProcessRuntime


class PrivateRuntime(Linux):
    def __init__(self, system):
        self.system = system
        self.processes = ProcessRuntime()
        self.queries = []
        self.loaded = 'not-found'
        self.active_state = 'inactive'
        self.pid = '0'

    def command(self, args, **kwargs):
        assert args[:3] == ['systemctl', 'show', '--property=LoadState,ActiveState,MainPID'], args
        self.queries.append(args)
        return f'LoadState={self.loaded}\nActiveState={self.active_state}\nMainPID={self.pid}\n'.encode()

    def preflight(self, host):
        self.private_preflight(host, self.system)


class PrivateReleases(unittest.TestCase):
    def setUp(self):
        self.fixture = test_instances.ExecutableReleases()
        self.fixture.setUp()
        self.addCleanup(self.fixture.tearDown)
        self.root = self.fixture.root
        self.system = self.root / 'system'
        self.system.mkdir()
        self.runtime = PrivateRuntime(self.system)
        self.machine = Machine(self.runtime)

    def request(self, version='v1', body=b'one', unit='services.compiler'):
        request = self.fixture.request(version, body, unit)
        host = request['host']
        host.update(edge=None, edge_sha256=None, caddy_sha256=None, caddy_version=None,
                    routes={}, trust={'prerequisites': {}, 'tunnel': None, 'architecture': 'x86_64'})
        request.pop('caddy')
        request['operation'] = digest(canonical([version, host]))
        return request

    def deploy(self, request):
        self.machine.run(request)
        return self.machine.run({**request, 'action': 'activate'})

    def test_private_update_and_rollback_never_start_an_edge(self):
        first = self.request()
        self.deploy(first)
        self.deploy(self.request('v2', b'two'))
        result = self.deploy({**first, 'rollback': True})

        self.assertEqual(result['phase'], 'complete')
        self.assertEqual(set(self.runtime.processes.running), {self.fixture.release('v1')})
        self.assertEqual(self.runtime.processes.enabled, {self.fixture.release('v1')})
        self.assertTrue(result['observations']['services.compiler'])
        for edge in (self.root / 'edge/releases').iterdir():

            self.assertEqual({p.name for p in edge.iterdir()}, {'config.json', 'template.json'})
            self.assertEqual((edge / 'config.json').read_bytes(), b'null')
        self.assertFalse(hasattr(self.runtime, 'user'))
        self.assertTrue(self.runtime.queries)

    def test_selected_private_update_preserves_other_process_and_history(self):
        first = self.request()
        other = self.request('other', b'other', 'services.worker')
        self.deploy(first)
        self.deploy(other)
        other_release = self.fixture.release('other', 'services.worker')
        other_contract = copy.deepcopy(self.runtime.processes.running[other_release])
        other_journal = self.root / 'units/services.worker/journal.json'
        before = other_journal.read_bytes()
        mark = len(self.runtime.processes.events)
        self.deploy(self.request('v2', b'two'))
        self.deploy({**first, 'rollback': True})

        self.assertEqual(self.runtime.processes.running[other_release], other_contract)
        self.assertEqual(other_journal.read_bytes(), before)
        self.assertTrue(other_release in self.runtime.processes.enabled)
        self.assertFalse(any(path == other_release for _, path in self.runtime.processes.events[mark:]))

    def test_private_runtime_failure_restores_previous_process(self):
        self.deploy(self.request())
        request = self.request('v2', b'two')
        def fail_after_metadata_promotion(point):
            if point == 'after_restart':
                self.runtime.processes.rejected.add('v2')
        machine = Machine(self.runtime, fail_after_metadata_promotion)
        machine.run(request)
        with self.assertRaisesRegex(ValueError, 'application readiness failed'):
            machine.run({**request, 'action': 'activate'})

        self.assertEqual(set(self.runtime.processes.running), {self.fixture.release('v1')})
        self.assertEqual(pointer(self.root / 'units/services.compiler/current'), 'releases/v1')
        state = json.loads((self.root / 'journal.json').read_bytes())

        self.assertEqual(state['phase'], 'reverted')

    def test_private_recovery_at_every_promotion_phase(self):
        points = ('installing', 'installed:services.compiler', 'candidates_started', 'prepared',
                  'before_units', 'switched:services.compiler', 'before_edge', 'after_edge',
                  'after_restart', 'before_retirement', 'after_retirement', 'complete')
        for index, point in enumerate(points):
            with self.subTest(point=point):
                previous = self.request('base' + str(index), b'previous')
                self.deploy(previous)
                request = self.request('next' + str(index), b'next')
                def interrupt(at):
                    if at == point:
                        raise Interrupted()
                machine = Machine(self.runtime, interrupt)
                with self.assertRaises(Interrupted):
                    machine.run(request)
                    machine.run({**request, 'action': 'activate'})
                result = self.deploy(request)

                self.assertEqual(result['phase'], 'complete')
                self.assertEqual(set(self.runtime.processes.running), {self.fixture.release(request['version'])})

    def test_private_declaration_rejects_edge_inputs_and_nonexecutables(self):
        changes = [(('host', field), 'unexpected') for field in
                   ('routes', 'caddy_sha256', 'caddy_version', 'edge_sha256')]
        changes.extend([
            (('caddy',), 'unexpected'),
            (('host', 'trust', 'prerequisites'), {'/etc/leptoskit/cert': 'pin'}),
            (('host', 'trust', 'tunnel'), {'unit': 'other.service'}),
            (('host', 'executables'), {}),
        ])

        for path, value in changes:
            with self.subTest(path=path):
                request = self.request()
                target = request
                for part in path[:-1]:
                    target = target[part]
                target[path[-1]] = value

                with self.assertRaises(ValueError):
                    self.machine.run(request)

                self.assertFalse((self.root / 'journal.json').exists())
                self.assertFalse(self.runtime.processes.events)
                self.assertFalse(self.runtime.queries)

    def test_private_metadata_cannot_gain_a_runtime_after_prepare(self):
        for target in ('caddy', 'config', 'mode'):
            with self.subTest(target=target):
                request = self.request('private' + target)
                self.machine.run(request)
                state = json.loads((self.root / 'journal.json').read_bytes())
                edge = self.root / 'edge/releases' / state['transaction']
                if target == 'caddy':
                    (edge / 'caddy').write_bytes(b'unrequested')
                elif target == 'config':
                    (edge / 'config.json').write_bytes(b'{}')
                else:
                    (edge / 'template.json').chmod(0o600)
                with self.assertRaises(ValueError):
                    self.machine.run({**request, 'action': 'activate'})

                self.assertIsNone(pointer(self.root / 'units/services.compiler/current'))
                self.assertFalse(self.runtime.processes.enabled)

    def test_public_private_role_change_requires_a_separate_migration(self):
        self.deploy(self.request())
        request = self.fixture.request('public', b'public')
        request['host']['trust'] = {'prerequisites': {}, 'tunnel': None}
        before = (self.root / 'journal.json').read_bytes()
        events = list(self.runtime.processes.events)
        with self.assertRaisesRegex(ValueError, 'shared edge routes'):
            self.machine.run(request)

        self.assertEqual((self.root / 'journal.json').read_bytes(), before)
        self.assertEqual(self.runtime.processes.events, events)

    def test_public_host_cannot_silently_become_private(self):
        public = self.fixture.request()
        self.fixture.deploy(public)
        self.runtime.processes = self.fixture.runtime.processes
        before = (self.root / 'journal.json').read_bytes()
        events = list(self.runtime.processes.events)
        with self.assertRaisesRegex(ValueError, 'shared edge routes'):
            self.machine.run(self.request('private', b'private'))

        self.assertEqual((self.root / 'journal.json').read_bytes(), before)
        self.assertEqual(self.runtime.processes.events, events)

    def test_private_preflight_rejects_existing_or_loaded_public_unit(self):
        host = self.request()['host']
        path = self.system / self.runtime.service(host)
        path.write_bytes(b'unrelated existing unit')
        with self.assertRaisesRegex(ValueError, 'existing edge unit'):
            self.runtime.preflight(host)
        path.unlink()
        for loaded, active, pid in [('loaded', 'inactive', '0'), ('not-found', 'active', '123'),
                                    ('not-found', 'inactive', '123')]:
            self.runtime.loaded, self.runtime.active_state, self.runtime.pid = loaded, active, pid
            with self.assertRaisesRegex(ValueError, 'loaded or active edge unit'):
                self.runtime.preflight(host)

        self.assertFalse(self.runtime.processes.events)

    def test_linux_private_preflight_does_not_require_an_edge_account(self):
        host = self.request()['host']
        with patch('host.sys.platform', 'linux'), patch('host.os.geteuid', return_value=0), \
             patch('host.os.uname', return_value=types.SimpleNamespace(machine='x86_64')), \
             patch('host.pwd.getpwnam', side_effect=AssertionError('edge account requested')):
            Linux.preflight(self.runtime, host)

        self.assertFalse(hasattr(self.runtime, 'user'))


if __name__ == '__main__':
    unittest.main()
