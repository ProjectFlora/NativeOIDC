"""Tests for scripts/android_maven_central_release.py.

A local HTTP server plays Maven Central (HEAD/GET of the publication's
files) and the Central Portal (POST /api/v1/publisher/status and
/api/v1/publisher/upload), and a fake GitHub CLI keeps the deployment record
(the Release assets maven-central-deployment-<n>.json) in a directory. Every row
of the decision table in docs/releasing.md is covered, plus the upload
sequence and --read-only (Issue #180).

Run with: python3 -m unittest discover -s scripts -p 'test_*.py'
"""
import hashlib
import http.server
import io
import json
import os
import sys
import tempfile
import threading
import unittest
import zipfile
from contextlib import redirect_stderr, redirect_stdout
from unittest import mock

SCRIPTS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPTS_DIR)
import android_maven_central_release as release  # noqa: E402

VERSION = "1.2.3"
AAR = b"the release aar"
AAR_SHA256 = hashlib.sha256(AAR).hexdigest()
OTHER_SHA256 = hashlib.sha256(b"another build").hexdigest()
BASE = "/maven2/net/project-flora/nativeoidc/native-oidc-android/%s/" % VERSION
DEPLOYMENT_ID = "11111111-2222-3333-4444-555555555555"

# A fake `gh` for the record: `release view` lists the assets in FAKE_GH_DIR,
# `release download` copies the record out, `release upload` copies it in.
# Like GitHub without --clobber, an upload of an existing asset name fails.
# FAKE_GH_FAIL_UPLOAD makes uploads fail; FAKE_GH_UPLOADS_ALLOWED makes every
# upload after that many successful ones fail; FAKE_GH_LOG lists every call.
FAKE_GH = r'''
import json, os, shutil, sys
store = os.environ["FAKE_GH_DIR"]
with open(os.environ["FAKE_GH_LOG"], "a") as log:
    log.write(json.dumps(sys.argv[1:]) + "\n")
args = sys.argv[1:]
if os.environ.get("FAKE_GH_NO_RELEASE"):
    sys.exit("release not found")
if args[:2] == ["release", "view"]:
    for name in sorted(os.listdir(store)):
        print(name)
elif args[:2] == ["release", "download"]:
    shutil.copy(os.path.join(store, args[args.index("--pattern") + 1]), args[args.index("--dir") + 1])
elif args[:2] == ["release", "upload"]:
    if "--clobber" in args:
        sys.exit("--clobber deletes the existing asset first")
    allowed = os.environ.get("FAKE_GH_UPLOADS_ALLOWED")
    with open(os.environ["FAKE_GH_LOG"]) as log:
        earlier = sum(1 for line in log if json.loads(line)[:2] == ["release", "upload"]) - 1
    if os.environ.get("FAKE_GH_FAIL_UPLOAD") or (allowed is not None and earlier >= int(allowed)):
        sys.exit("upload refused")
    if os.path.exists(os.path.join(store, os.path.basename(args[3]))):
        sys.exit("asset already exists")
    shutil.copy(args[3], store)
else:
    sys.exit("unexpected gh call %r" % args)
'''


class FastHTTPServer(http.server.HTTPServer):
    """Skip HTTPServer.server_bind's socket.getfqdn reverse lookup, which can stall."""

    def server_bind(self):
        import socketserver
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


class MockState:
    def __init__(self):
        self.central = {}  # file name -> bytes; a missing name answers 404
        self.central_after = None  # (status checks until visible, files) for a sync in progress
        self.central_checks = 0
        self.statuses = {}  # deployment id -> list of answers, the last one repeats
        self.upload_answer = (201, DEPLOYMENT_ID)
        self.requests = []  # (method, path)


def publish_everything(state, aar=AAR):
    for name in release.central_files(VERSION):
        state.central[name] = aar if name.endswith(".aar") else b"x"


class Handler(http.server.BaseHTTPRequestHandler):
    state = None

    def log_message(self, *args):
        pass

    def _central(self, send_body):
        name = self.path[len(BASE):]
        content = self.state.central.get(name)
        if content is None:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        if send_body:
            self.wfile.write(content)

    def do_HEAD(self):
        self.state.requests.append(("HEAD", self.path))
        if self.path.endswith(".aar"):
            self.state.central_checks += 1
            if self.state.central_after and self.state.central_checks >= self.state.central_after:
                publish_everything(self.state)
        self._central(False)

    def do_GET(self):
        self.state.requests.append(("GET", self.path))
        self._central(True)

    def _answer(self, status, body):
        payload = body.encode() if isinstance(body, str) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        self.state.requests.append(("POST", self.path))
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        if self.headers.get("Authorization") != "Bearer dXNlcjp0b2tlbg==":
            self._answer(401, "unauthorized")
        elif self.path.startswith("/api/v1/publisher/status?id="):
            deployment_id = self.path.split("=", 1)[1]
            answers = self.state.statuses.get(deployment_id)
            if not answers:
                self._answer(404, "not found")
                return
            answer = answers.pop(0) if len(answers) > 1 else answers[0]
            if isinstance(answer, int):
                self._answer(answer, "server error")
            else:
                self._answer(200, {"deploymentId": deployment_id, **answer})
        elif self.path.startswith("/api/v1/publisher/upload?"):
            assert b'name="bundle"' in body
            self._answer(*self.state.upload_answer)
        else:
            self._answer(404, "unknown")

    def do_DELETE(self):
        self.state.requests.append(("DELETE", self.path))
        self._answer(204, "")


class ReleaseTestCase(unittest.TestCase):
    def setUp(self):
        self.state = MockState()
        handler = type("BoundHandler", (Handler,), {"state": self.state})
        self.server = FastHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.store = os.path.join(self.tmp.name, "release")
        os.mkdir(self.store)
        fake_gh = os.path.join(self.tmp.name, "fake_gh.py")
        with open(fake_gh, "w") as handle:
            handle.write(FAKE_GH)
        self.gh_log = os.path.join(self.tmp.name, "gh.log")
        open(self.gh_log, "w").close()
        self.summary = os.path.join(self.tmp.name, "summary.md")
        self.output = os.path.join(self.tmp.name, "output")
        url = "http://127.0.0.1:%d" % self.server.server_port
        self.env = {
            "MAVEN_CENTRAL_BASE_URL": url + "/maven2",
            "CENTRAL_PORTAL_BASE_URL": url,
            "MAVEN_USER": "user",
            "MAVEN_PASSWORD": "token",
            "GH_COMMAND": json.dumps([sys.executable, fake_gh]),
            "FAKE_GH_DIR": self.store,
            "FAKE_GH_LOG": self.gh_log,
            "HTTP_TIMEOUT": "5",
            "PORTAL_RETRIES": "2",
            "PORTAL_RETRY_INTERVAL": "0",
            "DEPLOYMENT_WAIT_ATTEMPTS": "3",
            "DEPLOYMENT_WAIT_INTERVAL": "0",
            "CENTRAL_WAIT_ATTEMPTS": "3",
            "CENTRAL_WAIT_INTERVAL": "0",
            "GITHUB_OUTPUT": self.output,
            "GITHUB_STEP_SUMMARY": self.summary,
            "GITHUB_RUN_ID": "99",
            "GITHUB_RUN_ATTEMPT": "1",
        }

    def run_script(self, *args, extra_env=None):
        env = dict(self.env, **(extra_env or {}))
        stdout, stderr = io.StringIO(), io.StringIO()
        with mock.patch.dict(os.environ, env), redirect_stdout(stdout), redirect_stderr(stderr):
            code = release.main([*args])
        return code, stdout.getvalue(), stderr.getvalue()

    def decide(self, *extra, **kwargs):
        return self.run_script("decide", VERSION, "--expected-sha256", AAR_SHA256, *extra, **kwargs)

    def snapshots(self):
        return sorted(
            int(match.group(1)) for match in map(release.RECORD_PATTERN.match, os.listdir(self.store)) if match
        )

    def write_record(self, *attempts):
        seq = max(self.snapshots(), default=0) + 1
        with open(os.path.join(self.store, release.record_asset(seq)), "w") as handle:
            json.dump({"version": VERSION, "attempts": list(attempts)}, handle)

    def read_record(self, seq=None):
        seq = seq or max(self.snapshots())
        with open(os.path.join(self.store, release.record_asset(seq))) as handle:
            return json.load(handle)

    def gh_calls(self):
        with open(self.gh_log) as handle:
            return [json.loads(line) for line in handle]

    def decision(self):
        with open(self.output) as handle:
            return handle.read()

    def bundle(self, aar=AAR):
        path = os.path.join(self.tmp.name, "bundle.zip")
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr(BASE[len("/maven2/"):] + "native-oidc-android-%s.aar" % VERSION, aar)
        return path

    def portal_requests(self):
        return [request for request in self.state.requests if request[1].startswith("/api/")]

    def assert_no_mutation(self):
        self.assertFalse([r for r in self.state.requests if r[0] == "DELETE" or "/upload" in r[1]])
        self.assertFalse([r for r in self.state.requests if r[0] == "POST" and "/deployment/" in r[1]])


class DecideTest(ReleaseTestCase):
    def test_complete_publication_with_matching_aar_is_verified(self):
        publish_everything(self.state)
        code, out, _ = self.decide()
        self.assertEqual(code, 0)
        self.assertIn("decision=verified", self.decision())
        self.assertEqual(self.portal_requests(), [])
        self.assertEqual(self.gh_calls(), [])

    def test_complete_publication_with_other_aar_fails(self):
        publish_everything(self.state, aar=b"another build")
        code, _, err = self.decide()
        self.assertEqual(code, 1)
        self.assertIn("different bytes", err)

    def test_partial_publication_without_record_uploads(self):
        publish_everything(self.state)
        del self.state.central["native-oidc-android-%s.aar.asc" % VERSION]
        code, _, _ = self.decide()
        self.assertEqual(code, 0)
        self.assertIn("decision=upload", self.decision())
        self.assertEqual(self.portal_requests(), [])

    def test_no_release_means_no_record(self):
        code, _, _ = self.decide("--no-record")
        self.assertEqual(code, 0)
        self.assertIn("decision=upload", self.decision())
        self.assertEqual(self.gh_calls(), [])

    def test_release_lookup_failure_fails_closed(self):
        code, _, err = self.decide(extra_env={"FAKE_GH_NO_RELEASE": "1"})
        self.assertEqual(code, 1)
        self.assertIn("Could not list", err)

    def test_upload_started_without_id_fails_closed(self):
        self.write_record({"state": "upload-started", "runId": "7", "runAttempt": "1"})
        code, _, err = self.decide()
        self.assertEqual(code, 1)
        self.assertIn("never recorded", err)
        self.assertEqual(self.portal_requests(), [])

    def test_in_flight_deployment_published_and_synced_is_verified(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        self.state.statuses[DEPLOYMENT_ID] = [
            {"deploymentState": "VALIDATING"},
            {"deploymentState": "PUBLISHING"},
            {"deploymentState": "PUBLISHED"},
        ]
        self.state.central_after = 2  # visible on the second gate check
        code, _, err = self.decide()
        self.assertEqual(code, 0, err)
        self.assertIn("decision=verified", self.decision())
        self.assert_no_mutation()

    def test_published_deployment_that_never_syncs_fails_with_rerun_guidance(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "PUBLISHED"}]
        code, _, err = self.decide()
        self.assertEqual(code, 1)
        self.assertIn("still propagating", err)

    def test_deployment_still_in_flight_after_the_wait_fails(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "PUBLISHING"}]
        code, _, err = self.decide()
        self.assertEqual(code, 1)
        self.assertIn("still PUBLISHING", err)
        self.assertEqual(len(self.portal_requests()), 3)

    def test_validated_deployment_that_stays_validated_fails_closed(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "VALIDATED"}]
        code, _, err = self.decide()
        self.assertEqual(code, 1)
        self.assertIn("manual publish", err)
        self.assert_no_mutation()

    def test_failed_deployment_is_marked_and_uploaded_again_without_drop(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "FAILED", "errors": {"x": ["bad"]}}]
        code, _, _ = self.decide()
        self.assertEqual(code, 0)
        self.assertIn("decision=upload", self.decision())
        self.assertEqual(self.read_record()["attempts"][0]["state"], "failed")
        self.assert_no_mutation()

    def test_failed_latest_attempt_uploads_again(self):
        self.write_record({"state": "failed", "deploymentId": DEPLOYMENT_ID})
        code, _, _ = self.decide()
        self.assertEqual(code, 0)
        self.assertIn("decision=upload", self.decision())
        self.assertEqual(self.portal_requests(), [])

    def test_too_many_failed_attempts_fail_closed(self):
        self.write_record(*[{"state": "failed", "deploymentId": str(n)} for n in range(3)])
        code, _, err = self.decide()
        self.assertEqual(code, 1)
        self.assertIn("not transient", err)

    def test_unknown_deployment_fails_closed(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        code, _, err = self.decide()
        self.assertEqual(code, 1)
        self.assertIn("HTTP 404", err)

    def test_portal_server_errors_fail_closed_after_retries(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        self.state.statuses[DEPLOYMENT_ID] = [500]
        code, _, err = self.decide()
        self.assertEqual(code, 1)
        self.assertIn("did not answer", err)
        self.assertEqual(len(self.portal_requests()), 2)

    def test_portal_error_then_success_is_retried(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        self.state.statuses[DEPLOYMENT_ID] = [503, {"deploymentState": "PUBLISHED"}]
        publish_everything(self.state)
        self.state.central.pop("native-oidc-android-%s.pom" % VERSION)
        self.state.central_after = 2  # visible once the portal answered
        code, _, err = self.decide()
        self.assertEqual(code, 0, err)

    def test_portal_unreachable_fails_closed(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        code, _, err = self.decide(extra_env={"CENTRAL_PORTAL_BASE_URL": "http://127.0.0.1:9"})
        self.assertEqual(code, 1)
        self.assertIn("did not answer", err)

    def test_read_only_never_writes_or_mutates(self):
        self.write_record({"state": "uploaded", "deploymentId": DEPLOYMENT_ID})
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "FAILED", "errors": {}}]
        code, _, _ = self.decide("--read-only")
        self.assertEqual(code, 0)
        self.assertIn("decision=upload", self.decision())
        self.assertEqual(self.read_record()["attempts"][0]["state"], "uploaded")
        self.assertFalse([call for call in self.gh_calls() if call[1] == "upload"])
        self.assert_no_mutation()
        self.assertTrue(all(r[0] in ("GET", "HEAD") or "/status?" in r[1] for r in self.state.requests))


class UploadTest(ReleaseTestCase):
    def upload(self, *extra, **kwargs):
        return self.run_script(
            "upload", VERSION, "--expected-sha256", AAR_SHA256, "--bundle", self.bundle(), *extra, **kwargs
        )

    def test_upload_records_before_posting_and_records_the_id(self):
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "VALIDATING"}, {"deploymentState": "PUBLISHING"}]
        code, _, err = self.upload()
        self.assertEqual(code, 0, err)
        self.assertIn("decision=uploaded", self.decision())
        attempt = self.read_record()["attempts"][0]
        self.assertEqual(attempt["state"], "uploaded")
        self.assertEqual(attempt["deploymentId"], DEPLOYMENT_ID)
        self.assertEqual(attempt["aarSha256"], AAR_SHA256)
        self.assertEqual(attempt["runId"], "99")
        uploads = [call for call in self.gh_calls() if call[1] == "upload"]
        self.assertEqual(len(uploads), 2)  # upload-started, then uploaded
        upload_posts = [r for r in self.state.requests if "/upload?" in r[1]]
        self.assertEqual(len(upload_posts), 1)
        self.assertIn("publishingType=AUTOMATIC", upload_posts[0][1])

    def test_record_write_failure_prevents_the_upload(self):
        code, _, err = self.upload(extra_env={"FAKE_GH_FAIL_UPLOAD": "1"})
        self.assertEqual(code, 1)
        self.assertIn("Could not write", err)
        self.assertFalse([r for r in self.state.requests if "/upload?" in r[1]])

    def test_snapshots_are_appended_and_never_replaced(self):
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "PUBLISHING"}]
        code, _, err = self.upload()
        self.assertEqual(code, 0, err)
        self.assertEqual(self.snapshots(), [1, 2])
        self.assertEqual(self.read_record(1)["attempts"][0]["state"], "upload-started")
        self.assertEqual(self.read_record(2)["attempts"][0]["state"], "uploaded")

    def test_lost_id_write_after_accepted_upload_never_uploads_again(self):
        # The Portal accepts the bundle, then writing the ID fails: the
        # upload-started snapshot must survive, so a re-run fails closed.
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "PUBLISHING"}]
        code, _, err = self.upload(extra_env={"FAKE_GH_UPLOADS_ALLOWED": "1"})
        self.assertEqual(code, 1)
        self.assertIn("Could not write", err)
        self.assertEqual(self.read_record()["attempts"][-1]["state"], "upload-started")
        code, _, err = self.upload()
        self.assertEqual(code, 1)
        self.assertIn("never recorded", err)
        self.assertEqual(len([r for r in self.state.requests if "/upload?" in r[1]]), 1)

    def test_failed_validation_after_upload_is_recorded(self):
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "FAILED", "errors": {"sig": ["bad"]}}]
        code, _, err = self.upload()
        self.assertEqual(code, 1)
        self.assertIn("failed validation", err)
        self.assertEqual(self.read_record()["attempts"][0]["state"], "failed")

    def test_refused_upload_is_recorded_as_failed(self):
        self.state.upload_answer = (400, "bad bundle")
        code, _, err = self.upload()
        self.assertEqual(code, 1)
        self.assertIn("refused the upload", err)
        self.assertEqual(self.read_record()["attempts"][0]["state"], "failed")

    def test_upload_resumes_after_a_failed_attempt(self):
        self.write_record({"state": "failed", "deploymentId": "old"})
        self.state.statuses[DEPLOYMENT_ID] = [{"deploymentState": "PUBLISHED"}]
        code, _, err = self.upload()
        self.assertEqual(code, 0, err)
        attempts = self.read_record()["attempts"]
        self.assertEqual([a["state"] for a in attempts], ["failed", "uploaded"])

    def test_no_upload_when_already_published(self):
        publish_everything(self.state)
        code, _, _ = self.upload()
        self.assertEqual(code, 0)
        self.assertIn("decision=verified", self.decision())
        self.assertFalse([r for r in self.state.requests if "/upload?" in r[1]])

    def test_no_upload_when_upload_was_started_but_not_recorded(self):
        self.write_record({"state": "upload-started"})
        code, _, _ = self.upload()
        self.assertEqual(code, 1)
        self.assertFalse([r for r in self.state.requests if "/upload?" in r[1]])

    def test_bundle_with_other_aar_is_refused(self):
        code, _, err = self.run_script(
            "upload", VERSION, "--expected-sha256", OTHER_SHA256, "--bundle", self.bundle()
        )
        self.assertEqual(code, 1)
        self.assertIn("bundle's AAR", err)
        self.assertEqual(self.state.requests, [])

    def test_read_only_refuses_upload(self):
        code, _, err = self.upload("--read-only")
        self.assertEqual(code, 1)
        self.assertIn("never uploads", err)
        self.assertEqual(self.state.requests, [])


class VerifyAndProbeTest(ReleaseTestCase):
    def test_verify_passes_for_complete_matching_publication(self):
        publish_everything(self.state)
        code, _, _ = self.run_script("verify", VERSION, "--expected-sha256", AAR_SHA256)
        self.assertEqual(code, 0)

    def test_verify_fails_for_incomplete_publication(self):
        code, _, err = self.run_script("verify", VERSION, "--expected-sha256", AAR_SHA256)
        self.assertEqual(code, 1)
        self.assertIn("not visible", err)

    def test_probe_accepts_not_found_for_unknown_deployment(self):
        code, _, _ = self.run_script("probe", VERSION, "--read-only")
        self.assertEqual(code, 0)
        self.assertEqual(self.portal_requests(), [("POST", "/api/v1/publisher/status?id=" + release.PROBE_DEPLOYMENT_ID)])

    def test_probe_fails_for_rejected_credentials(self):
        code, _, err = self.run_script("probe", VERSION, "--read-only", extra_env={"MAVEN_PASSWORD": "wrong"})
        self.assertEqual(code, 1)
        self.assertIn("rejected the credentials", err)

    def test_summary_records_the_evidence(self):
        publish_everything(self.state)
        self.decide("--read-only")
        with open(self.summary) as handle:
            summary = handle.read()
        self.assertIn("(read-only)", summary)
        self.assertIn("all 10 files visible", summary)
        self.assertIn("**Result:** verified", summary)


if __name__ == "__main__":
    unittest.main()
