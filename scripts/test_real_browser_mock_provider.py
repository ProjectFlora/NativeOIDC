"""Tests for scripts/real_browser_mock_provider.py.

The provider drives the real system-browser verification
(docs/real-browser-verification.md), so a broken provider would make a
manual or CI run fail for reasons unrelated to the browser. These tests run
the flow with a plain HTTP client standing in for the browser and the
library.
"""

import base64
import hashlib
import http.client
import json
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from urllib.parse import parse_qs, urlencode, urlsplit

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import real_browser_mock_provider as provider_module  # noqa: E402

SCRIPT = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "real_browser_mock_provider.py"
)
VERIFIER = "real-browser-verifier-0123456789-abcdefghijklmnopqrstuvwxyz"
CALLBACK = "http://127.0.0.1:49152/oauth/callback"


def challenge(verifier):
    digest = hashlib.sha256(verifier.encode("ascii")).digest()
    return base64.urlsafe_b64encode(digest).rstrip(b"=").decode("ascii")


def authorization_query(**overrides):
    parameters = {
        "client_id": provider_module.CLIENT_ID,
        "response_type": "code",
        "scope": provider_module.SCOPE,
        "code_challenge_method": "S256",
        "code_challenge": challenge(VERIFIER),
        "state": "state-value",
        "redirect_uri": CALLBACK,
    }
    parameters.update(overrides)
    return urlencode({k: v for k, v in parameters.items() if v is not None})


def token_body(**overrides):
    parameters = {
        "grant_type": "authorization_code",
        "code": provider_module.AUTHORIZATION_CODE,
        "client_id": provider_module.CLIENT_ID,
        "redirect_uri": CALLBACK,
        "code_verifier": VERIFIER,
    }
    parameters.update(overrides)
    return urlencode({k: v for k, v in parameters.items() if v is not None})


class ProviderTestCase(unittest.TestCase):
    def setUp(self):
        self.provider = provider_module.Provider()
        self.thread = threading.Thread(target=self.provider.serve_forever, daemon=True)
        self.thread.start()

    def tearDown(self):
        self.provider.shutdown()
        self.provider.server_close()

    def request(self, method, path, body=None, headers=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.provider.server_port, timeout=5)
        try:
            connection.request(method, path, body=body, headers=headers or {})
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            connection.close()

    def authorize(self, query=None, user_agent="TestBrowser/1.0"):
        return self.request(
            "GET",
            "/authorize?" + (query or authorization_query()),
            headers={"User-Agent": user_agent},
        )

    def token(self, body=None):
        return self.request(
            "POST",
            "/token",
            body=body or token_body(),
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )

    def test_completes_the_flow_and_records_the_browser(self):
        status, headers, _ = self.authorize()
        self.assertEqual(status, 302)
        location = urlsplit(headers["Location"])
        self.assertEqual(
            f"{location.scheme}://{location.netloc}{location.path}", CALLBACK
        )
        self.assertEqual(
            parse_qs(location.query),
            {"code": [provider_module.AUTHORIZATION_CODE], "state": ["state-value"]},
        )

        status, headers, body = self.token()
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(body)["access_token"], provider_module.ACCESS_TOKEN)
        self.assertTrue(self.provider.completed.is_set())
        result = self.provider.result()
        self.assertTrue(result["completed"])
        self.assertEqual(result["authorization_user_agent"], "TestBrowser/1.0")
        self.assertEqual(result["redirect_uri"], CALLBACK)

    def test_unrelated_browser_requests_are_not_errors(self):
        status, _, _ = self.request("GET", "/favicon.ico")
        self.assertEqual(status, 404)
        self.assertEqual(self.provider.result()["errors"], [])

    def test_rejects_authorization_requests_for_other_redirects_or_clients(self):
        cases = [
            authorization_query(redirect_uri="http://localhost:49152/oauth/callback"),
            authorization_query(redirect_uri="http://127.0.0.1/oauth/callback"),
            authorization_query(redirect_uri="http://127.0.0.1:49152/other"),
            authorization_query(redirect_uri="https://127.0.0.1:49152/oauth/callback"),
            authorization_query(client_id="other"),
            authorization_query(code_challenge_method="plain"),
            authorization_query(state=None),
        ]
        for query in cases:
            with self.subTest(query=query):
                status, _, _ = self.authorize(query)
                self.assertEqual(status, 400)
        result = self.provider.result()
        self.assertFalse(result["authorization_requested"])
        self.assertEqual(len(result["errors"]), len(cases))

    def test_rejects_a_second_authorization_request(self):
        self.assertEqual(self.authorize()[0], 302)
        self.assertEqual(self.authorize()[0], 400)
        self.assertIn("duplicate authorization request", self.provider.result()["errors"])

    def test_rejects_token_requests_that_do_not_match_the_authorization(self):
        self.assertEqual(self.authorize()[0], 302)
        for body in [
            token_body(code_verifier="wrong-verifier-0123456789-abcdefghijklmnopqrstuvwxyz"),
            token_body(code="other"),
            token_body(redirect_uri="http://127.0.0.1:49153/oauth/callback"),
            token_body(client_id="other"),
            token_body(grant_type="refresh_token"),
        ]:
            with self.subTest(body=body):
                self.assertEqual(self.token(body)[0], 400)
        self.assertFalse(self.provider.completed.is_set())
        self.assertFalse(self.provider.result()["completed"])

    def test_rejects_a_token_request_before_authorization(self):
        self.assertEqual(self.token()[0], 400)
        self.assertFalse(self.provider.completed.is_set())


class CommandLineTestCase(unittest.TestCase):
    def test_reports_a_timeout_as_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            result_file = os.path.join(directory, "result.json")
            process = subprocess.run(
                [sys.executable, SCRIPT, "--result-file", result_file, "--timeout", "0.2"],
                capture_output=True,
                text=True,
                timeout=30,
            )
            self.assertEqual(process.returncode, 1)
            self.assertTrue(process.stdout.startswith("http://127.0.0.1:"))
            with open(result_file, encoding="utf-8") as file:
                result = json.load(file)
            self.assertFalse(result["completed"])
            self.assertIn("no token request within 0.2 seconds", result["errors"])

    def test_completes_from_the_command_line(self):
        with tempfile.TemporaryDirectory() as directory:
            result_file = os.path.join(directory, "result.json")
            process = subprocess.Popen(
                [sys.executable, SCRIPT, "--result-file", result_file, "--timeout", "30"],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                base = urlsplit(process.stdout.readline().strip())
                connection = http.client.HTTPConnection(base.hostname, base.port, timeout=5)
                connection.request("GET", "/authorize?" + authorization_query())
                self.assertEqual(connection.getresponse().status, 302)
                connection.close()
                connection = http.client.HTTPConnection(base.hostname, base.port, timeout=5)
                connection.request(
                    "POST",
                    "/token",
                    body=token_body(),
                    headers={"Content-Type": "application/x-www-form-urlencoded"},
                )
                self.assertEqual(connection.getresponse().status, 200)
                connection.close()
                self.assertEqual(process.wait(timeout=30), 0)
            finally:
                if process.poll() is None:
                    process.kill()
                process.communicate()
            with open(result_file, encoding="utf-8") as file:
                self.assertTrue(json.load(file)["completed"])


if __name__ == "__main__":
    unittest.main()
