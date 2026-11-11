#!/usr/bin/env python3
"""Deployment checks wait for service routes as well as the app's probe."""
import unittest
from unittest import mock

import services_deploy_checks as checks


class RouteReadiness(unittest.TestCase):
    def test_waits_for_delayed_service_routes_even_when_the_app_is_ready(self):
        clock = [0]
        calls = []
        analytics = iter([ConnectionRefusedError("not listening"), 503, 200])
        echo = iter([503, 200, 200])

        def request(base, path, method="GET", body=None):
            calls.append(path)
            if path == f"{checks.ANALYTICS}/api/summary":
                self.assertEqual((method, body), ("POST", {"days": 7}))
                status = next(analytics)
            elif path == f"{checks.ECHO}/headers":
                status = next(echo)
            else:
                self.assertEqual(path, "/readyz")
                status = 200
            if isinstance(status, OSError):
                raise status
            return status, {}, ""

        def sleep(delay):
            clock[0] += delay

        with mock.patch.object(checks, "request", request), \
                mock.patch.object(checks.time, "monotonic", lambda: clock[0]), \
                mock.patch.object(checks.time, "sleep", sleep):
            checks.wait_until_ready("http://edge", 10)

        self.assertEqual(clock[0], 4)
        self.assertEqual(calls.count(f"{checks.ANALYTICS}/api/summary"), 3)
        self.assertEqual(calls.count(f"{checks.ECHO}/headers"), 3)

    def test_a_persistently_unavailable_service_fails_with_its_route(self):
        clock = [0]

        def request(base, path, method="GET", body=None):
            return (503 if path == f"{checks.ANALYTICS}/api/summary" else 200), {}, ""

        def sleep(delay):
            clock[0] += delay

        with mock.patch.object(checks, "request", request), \
                mock.patch.object(checks.time, "monotonic", lambda: clock[0]), \
                mock.patch.object(checks.time, "sleep", sleep):
            with self.assertRaisesRegex(SystemExit, "analytics/api/summary answered 503"):
                checks.wait_until_ready("http://edge", 1)


if __name__ == "__main__":
    unittest.main()
