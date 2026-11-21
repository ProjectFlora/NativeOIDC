#!/usr/bin/env python3
"""Local OAuth provider for the real system-browser verification.

The provider serves one authorization code flow for a loopback redirect
(`http://127.0.0.1:<port>/oauth/callback`) without user interaction:

* `GET /authorize` validates the request and redirects the browser to the
  loopback callback with a code and the request's state. The request's
  `User-Agent` is recorded, so the result shows which program followed the
  authorization URL.
* `POST /token` validates the code, redirect URI, client ID, and PKCE
  verifier, and returns a token response.

The provider prints its base URL on the first standard output line, then
serves until the token request has been answered or `--timeout` seconds have
passed. It keeps a JSON result in `--result-file`, rewritten on every change
and marked `"final": true` when the provider exits normally, and exits with 0
only when the flow completed. See docs/real-browser-verification.md.
"""

import argparse
import base64
import hashlib
import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlencode, urlsplit, urlunsplit

CLIENT_ID = "real-browser-check"
SCOPE = "profile"
CALLBACK_PATH = "/oauth/callback"
AUTHORIZATION_CODE = "real-browser-authorization-code"
ACCESS_TOKEN = "real-browser-access-token"
MAX_BODY_SIZE = 16 * 1024


class Provider(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address=("127.0.0.1", 0), result_file=None):
        super().__init__(address, Handler)
        self.lock = threading.Lock()
        self.completed = threading.Event()
        self.result_file = result_file
        self.authorization = None
        self.authorization_user_agent = None
        self.token_served = False
        self.errors = []
        self.save()

    @property
    def base_url(self):
        return f"http://127.0.0.1:{self.server_port}"

    def record_error(self, message):
        with self.lock:
            self.errors.append(message)
        self.save()

    def save(self, final=False):
        """Writes the current result to the result file, if there is one.

        The file is rewritten on every change, so a provider that is killed
        before its deadline (as the run scripts do when the check fails
        first) still leaves the evidence it collected, with `final` false.
        """
        if self.result_file is None:
            return
        with self.lock:
            path = Path(self.result_file)
            path.parent.mkdir(parents=True, exist_ok=True)
            temporary = path.with_name(path.name + ".tmp")
            temporary.write_text(
                json.dumps(self._result(final), indent=2) + "\n", encoding="utf-8"
            )
            temporary.replace(path)

    def result(self, final=False):
        with self.lock:
            return self._result(final)

    def _result(self, final):
        return {
            "final": final,
            "completed": self.token_served and not self.errors,
            "authorization_requested": self.authorization is not None,
            "authorization_user_agent": self.authorization_user_agent,
            "redirect_uri": (self.authorization or {}).get("redirect_uri"),
            "token_served": self.token_served,
            "errors": list(self.errors),
        }


def is_loopback_callback(redirect_uri):
    try:
        redirect = urlsplit(redirect_uri)
        port = redirect.port
    except ValueError:
        return False
    return (
        redirect.scheme == "http"
        and redirect.hostname == "127.0.0.1"
        and port is not None
        and redirect.path == CALLBACK_PATH
        and not redirect.query
        and not redirect.fragment
        and redirect.username is None
    )


def s256(verifier):
    digest = hashlib.sha256(verifier.encode("ascii")).digest()
    return base64.urlsafe_b64encode(digest).rstrip(b"=").decode("ascii")


class Handler(BaseHTTPRequestHandler):
    server: Provider

    def do_GET(self):
        request = urlsplit(self.path)
        if request.path != "/authorize":
            # Browsers request other paths, such as /favicon.ico.
            self.respond(404, b"")
            return

        parameters = parse_qs(request.query, keep_blank_values=True)
        redirect_uri = single(parameters, "redirect_uri")
        state = single(parameters, "state")
        challenge = single(parameters, "code_challenge")
        if (
            single(parameters, "client_id") != CLIENT_ID
            or single(parameters, "response_type") != "code"
            or single(parameters, "scope") != SCOPE
            or single(parameters, "code_challenge_method") != "S256"
            or not state
            or not challenge
            or not redirect_uri
            or not is_loopback_callback(redirect_uri)
        ):
            self.server.record_error(f"invalid authorization request: {self.path}")
            self.respond(400, b"Invalid authorization request")
            return

        with self.server.lock:
            if self.server.authorization is not None:
                duplicate = True
            else:
                duplicate = False
                self.server.authorization = {
                    "redirect_uri": redirect_uri,
                    "challenge": challenge,
                }
                self.server.authorization_user_agent = self.headers.get("User-Agent")
        if duplicate:
            self.server.record_error("duplicate authorization request")
            self.respond(400, b"Duplicate authorization request")
            return
        self.server.save()

        callback = urlsplit(redirect_uri)._replace(
            query=urlencode({"code": AUTHORIZATION_CODE, "state": state})
        )
        self.respond(302, b"", {"Location": urlunsplit(callback)})

    def do_POST(self):
        if self.path != "/token":
            self.server.record_error(f"unexpected POST {self.path}")
            self.respond(404, b"")
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if length < 0 or length > MAX_BODY_SIZE:
                raise ValueError("invalid request size")
            body = self.rfile.read(length).decode("ascii")
            parameters = parse_qs(body, keep_blank_values=True, strict_parsing=True)
        except (UnicodeDecodeError, ValueError):
            self.server.record_error("unreadable token request")
            self.respond(400, b"Invalid token request")
            return

        verifier = single(parameters, "code_verifier")
        with self.server.lock:
            authorization = self.server.authorization
            already_served = self.server.token_served
        if (
            authorization is None
            or already_served
            or single(parameters, "grant_type") != "authorization_code"
            or single(parameters, "code") != AUTHORIZATION_CODE
            or single(parameters, "client_id") != CLIENT_ID
            or single(parameters, "redirect_uri") != authorization["redirect_uri"]
            or not verifier
            or s256(verifier) != authorization["challenge"]
        ):
            self.server.record_error("invalid token request")
            self.respond(400, b"Invalid token request")
            return

        self.respond(
            200,
            json.dumps(
                {
                    "access_token": ACCESS_TOKEN,
                    "token_type": "Bearer",
                    "expires_in": 3600,
                    "scope": SCOPE,
                }
            ).encode("ascii"),
            {"Content-Type": "application/json", "Cache-Control": "no-store"},
        )
        with self.server.lock:
            self.server.token_served = True
        self.server.save()
        self.server.completed.set()

    def respond(self, status, body, headers=None):
        self.send_response(status)
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format_string, *args):
        print(format_string % args, file=sys.stderr, flush=True)


def single(parameters, name):
    values = parameters.get(name, [])
    return values[0] if len(values) == 1 else None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--result-file", required=True, type=Path)
    parser.add_argument("--timeout", type=float, default=300.0)
    args = parser.parse_args(argv)

    provider = Provider(result_file=args.result_file)
    thread = threading.Thread(target=provider.serve_forever, daemon=True)
    thread.start()
    print(provider.base_url, flush=True)
    try:
        provider.completed.wait(args.timeout)
    finally:
        provider.shutdown()
        provider.server_close()

    if not provider.completed.is_set():
        provider.record_error(f"no token request within {args.timeout:g} seconds")
    provider.save(final=True)
    result = provider.result(final=True)
    print(json.dumps(result), file=sys.stderr, flush=True)
    return 0 if result["completed"] else 1


if __name__ == "__main__":
    sys.exit(main())
