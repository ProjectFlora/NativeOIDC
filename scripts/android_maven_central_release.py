#!/usr/bin/env python3
"""Publishes the Android library to Maven Central so that a failed run can resume.

The release workflow's publish_maven job uploads the signed bundle to the
Central Portal with this script instead of the Gradle plugin's
publishAndReleaseToMavenCentral, which neither reports nor keeps the
deployment ID. Each upload is recorded in the GitHub Release: an
"upload-started" attempt is written before the bundle is posted, and the
deployment ID is added once the Portal returns it. The record is
append-only: every write adds a new asset maven-central-deployment-<n>.json,
a full snapshot, without replacing an earlier one, and the highest <n> in
the "uploaded" state is the current record ("starter" assets, which a
failed upload can leave, are skipped, and their numbers are never reused).
A failed or interrupted write leaves the previous snapshot, so the "upload-started" marker cannot be lost (`gh release upload
--clobber` would delete the old asset before uploading the new one). A later
run (Re-run failed jobs) reads that record and
the state on Maven Central and the Portal, and either verifies the existing
publication, waits for a deployment in flight, or uploads, so a version is
never uploaded twice (Issue #180). Only documented Publisher API endpoints are
used: upload, status, and nothing that publishes or drops a deployment.

Subcommands:
  decide   Classify the version and print the decision: "verified" (the
           complete publication is on Maven Central with the expected AAR) or
           "upload" (no deployment exists, or every one has failed). Anything
           else fails.
  upload   Run decide, then upload the bundle if the decision is "upload",
           and wait until the Portal starts publishing it.
  verify   Check only that Maven Central serves the complete publication and
           the expected AAR.
  probe    Read-only check that the Portal accepts the credentials, with a
           status query for a deployment ID that does not exist.

With --read-only, only reads are made: GET/HEAD on Maven Central, the Portal
status query, and reading the record. The record is never written and
nothing is uploaded; "upload" is refused.

Environment (defaults in parentheses, overridable for tests):
  MAVEN_CENTRAL_BASE_URL    (https://repo.maven.apache.org/maven2)
  CENTRAL_PORTAL_BASE_URL   (https://central.sonatype.com)
  MAVEN_USER, MAVEN_PASSWORD  Central Portal user token
  GH_COMMAND                JSON list that runs the GitHub CLI (["gh"])
  HTTP_TIMEOUT              seconds per request (60)
  UPLOAD_TIMEOUT            seconds for the bundle upload (600)
  PORTAL_RETRIES            attempts per Portal query on 5xx or transport errors (3)
  PORTAL_RETRY_INTERVAL     seconds between them (10)
  DEPLOYMENT_WAIT_ATTEMPTS  status polls while a deployment is in flight (30)
  DEPLOYMENT_WAIT_INTERVAL  seconds between them (60)
  CENTRAL_WAIT_ATTEMPTS     Maven Central gate checks after PUBLISHED (30)
  CENTRAL_WAIT_INTERVAL     seconds between them (60)
  GITHUB_OUTPUT, GITHUB_STEP_SUMMARY, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT
"""

from __future__ import annotations

import argparse
import base64
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
import zipfile
from dataclasses import dataclass, field

GROUP = "net.project-flora.nativeoidc"
ARTIFACT = "native-oidc-android"
RECORD_PREFIX = "maven-central-deployment-"
RECORD_PATTERN = re.compile(r"^maven-central-deployment-([1-9][0-9]*)\.json$")
# More failed deployments than this means the failure is not transient.
MAX_FAILED_ATTEMPTS = 3
IN_FLIGHT_STATES = {"PENDING", "VALIDATING", "VALIDATED", "PUBLISHING"}
# The all-zero UUID never names a deployment; probe uses it.
PROBE_DEPLOYMENT_ID = "00000000-0000-0000-0000-000000000000"


class ReleaseError(Exception):
    """A state the job must not continue from; the message says what to do."""


def env_int(name: str, default: int) -> int:
    return int(os.environ.get(name, default))


def central_files(version: str) -> list[str]:
    """The publication's files: the set the availability gate checks.

    The same files as scripts/verify-android-maven-consumer.sh and
    scripts/check-android-maven-publication.sh.
    """
    base = f"{ARTIFACT}-{version}"
    files = [f"{base}.aar", f"{base}.pom", f"{base}.module", f"{base}-sources.jar", f"{base}-javadoc.jar"]
    return files + [f"{name}.asc" for name in files]


def central_path(version: str) -> str:
    return f"{GROUP.replace('.', '/')}/{ARTIFACT}/{version}"


def sha256_file(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def now() -> str:
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


@dataclass
class Context:
    version: str
    expected_sha256: str
    read_only: bool
    use_record: bool
    central_base: str = field(
        default_factory=lambda: os.environ.get("MAVEN_CENTRAL_BASE_URL", "https://repo.maven.apache.org/maven2").rstrip("/")
    )
    portal_base: str = field(
        default_factory=lambda: os.environ.get("CENTRAL_PORTAL_BASE_URL", "https://central.sonatype.com").rstrip("/")
    )
    evidence: list[str] = field(default_factory=list)
    # The <n> of the current record snapshot; 0 when there is none.
    record_seq: int = 0

    @property
    def tag(self) -> str:
        return f"v{self.version}"

    def note(self, line: str) -> None:
        print(line, flush=True)
        self.evidence.append(line)


# --- Maven Central -----------------------------------------------------------


def http_status(url: str, method: str = "HEAD") -> int:
    request = urllib.request.Request(url, method=method)
    try:
        with urllib.request.urlopen(request, timeout=env_int("HTTP_TIMEOUT", 60)) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code
    except (urllib.error.URLError, OSError):
        return 0


def central_gate(ctx: Context) -> dict[str, int]:
    base = f"{ctx.central_base}/{central_path(ctx.version)}"
    return {name: http_status(f"{base}/{name}") for name in central_files(ctx.version)}


def gate_summary(statuses: dict[str, int]) -> str:
    missing = [f"{name} {status}" for name, status in statuses.items() if status != 200]
    return ", ".join(missing) if missing else f"all {len(statuses)} files visible"


def verify_central_aar(ctx: Context) -> None:
    url = f"{ctx.central_base}/{central_path(ctx.version)}/{ARTIFACT}-{ctx.version}.aar"
    digest = hashlib.sha256()
    try:
        with urllib.request.urlopen(url, timeout=env_int("HTTP_TIMEOUT", 60)) as response:
            for chunk in iter(lambda: response.read(1 << 20), b""):
                digest.update(chunk)
    except (urllib.error.URLError, OSError) as error:
        raise ReleaseError(f"Could not download the AAR from Maven Central ({url}): {error}") from error
    actual = digest.hexdigest()
    if actual != ctx.expected_sha256:
        raise ReleaseError(
            f"The AAR on Maven Central has SHA-256 {actual}, but the GitHub Release asset has "
            f"{ctx.expected_sha256}. The two channels ship different bytes; this cannot be fixed by "
            "re-running. See 'Recovering from a failed run' in docs/releasing.md."
        )
    ctx.note(f"The AAR on Maven Central matches the GitHub Release asset (SHA-256 {actual}).")


def verify(ctx: Context) -> str:
    statuses = central_gate(ctx)
    ctx.note(f"Maven Central: {gate_summary(statuses)}.")
    if any(status != 200 for status in statuses.values()):
        raise ReleaseError(f"The complete publication of {ctx.version} is not visible on Maven Central.")
    verify_central_aar(ctx)
    return "verified"


def wait_for_central(ctx: Context) -> str:
    attempts = env_int("CENTRAL_WAIT_ATTEMPTS", 30)
    interval = env_int("CENTRAL_WAIT_INTERVAL", 60)
    statuses: dict[str, int] = {}
    for attempt in range(1, attempts + 1):
        statuses = central_gate(ctx)
        if all(status == 200 for status in statuses.values()):
            ctx.note(f"Maven Central: {gate_summary(statuses)}.")
            verify_central_aar(ctx)
            return "verified"
        print(f"Central sync check {attempt}/{attempts}: {gate_summary(statuses)}", flush=True)
        if attempt < attempts:
            time.sleep(interval)
    raise ReleaseError(
        f"The deployment is PUBLISHED, but Maven Central still lacks files after {attempts} checks "
        f"({gate_summary(statuses)}). The sync is still propagating: re-run the failed job later, "
        "or dispatch the release-consumer workflow once the files are visible."
    )


# --- Central Portal ----------------------------------------------------------


def portal_token() -> str:
    user = os.environ.get("MAVEN_USER", "")
    password = os.environ.get("MAVEN_PASSWORD", "")
    if not user or not password:
        raise ReleaseError("MAVEN_USER and MAVEN_PASSWORD (the Central Portal user token) are required.")
    return base64.b64encode(f"{user}:{password}".encode()).decode()


def portal_request(
    ctx: Context,
    method: str,
    path: str,
    *,
    body: bytes | None = None,
    content_type: str | None = None,
    timeout: int | None = None,
    retry: bool = True,
) -> tuple[int, bytes]:
    """Returns (status, body) for any HTTP answer; raises after transport failures or 5xx."""
    url = f"{ctx.portal_base}{path}"
    headers = {"Authorization": f"Bearer {portal_token()}", "User-Agent": "NativeOIDC-release"}
    if content_type:
        headers["Content-Type"] = content_type
    attempts = env_int("PORTAL_RETRIES", 3) if retry else 1
    last_error = ""
    for attempt in range(1, attempts + 1):
        request = urllib.request.Request(url, data=body, method=method, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=timeout or env_int("HTTP_TIMEOUT", 60)) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            payload = error.read()
            if error.code < 500:
                return error.code, payload
            last_error = f"HTTP {error.code}: {payload.decode(errors='replace')[:500]}"
        except (urllib.error.URLError, OSError) as error:
            last_error = str(error)
        print(f"Portal {method} {path} attempt {attempt}/{attempts} failed: {last_error}", file=sys.stderr, flush=True)
        if attempt < attempts:
            time.sleep(env_int("PORTAL_RETRY_INTERVAL", 10))
    raise ReleaseError(f"The Central Portal did not answer {method} {path}: {last_error}. Failing closed.")


def deployment_status(ctx: Context, deployment_id: str) -> dict:
    status, payload = portal_request(ctx, "POST", f"/api/v1/publisher/status?id={deployment_id}")
    if status != 200:
        raise ReleaseError(
            f"The Central Portal answered HTTP {status} for deployment {deployment_id}: "
            f"{payload.decode(errors='replace')[:500]}. Failing closed."
        )
    try:
        answer = json.loads(payload)
    except json.JSONDecodeError as error:
        raise ReleaseError(f"The Central Portal status of {deployment_id} is not JSON. Failing closed.") from error
    if not isinstance(answer, dict) or not isinstance(answer.get("deploymentState"), str):
        raise ReleaseError(f"The Central Portal status of {deployment_id} has no deploymentState. Failing closed.")
    return answer


def wait_while_in_flight(ctx: Context, deployment_id: str, done: set[str]) -> dict:
    """Polls the deployment until its state is in `done` or not in flight, or the wait ends."""
    attempts = env_int("DEPLOYMENT_WAIT_ATTEMPTS", 30)
    interval = env_int("DEPLOYMENT_WAIT_INTERVAL", 60)
    answer: dict = {}
    for attempt in range(1, attempts + 1):
        answer = deployment_status(ctx, deployment_id)
        state = answer["deploymentState"]
        print(f"Deployment {deployment_id} check {attempt}/{attempts}: {state}", flush=True)
        if state in done or state not in IN_FLIGHT_STATES:
            break
        if attempt < attempts:
            time.sleep(interval)
    return answer


# --- Deployment record (GitHub Release asset) --------------------------------


def gh(*args: str) -> subprocess.CompletedProcess:
    command = json.loads(os.environ.get("GH_COMMAND", '["gh"]'))
    return subprocess.run([*command, *args], capture_output=True, text=True)


def record_asset(seq: int) -> str:
    return f"{RECORD_PREFIX}{seq}.json"


def read_record(ctx: Context) -> dict | None:
    if not ctx.use_record:
        ctx.note("No GitHub Release exists for this version, so there is no deployment record.")
        return None
    listing = gh("release", "view", ctx.tag, "--json", "assets", "--jq", '.assets[] | "\\(.name) \\(.state)"')
    if listing.returncode != 0:
        raise ReleaseError(f"Could not list the assets of the GitHub Release {ctx.tag}: {listing.stderr.strip()}")
    complete: list[int] = []
    incomplete: list[int] = []
    for line in listing.stdout.splitlines():
        asset_name, _, state = line.strip().partition(" ")
        match = RECORD_PATTERN.match(asset_name)
        if match:
            (complete if state == "uploaded" else incomplete).append(int(match.group(1)))
    # The next snapshot takes a number no asset holds, incomplete ones included.
    ctx.record_seq = max(complete + incomplete, default=0)
    if incomplete:
        # A failed asset upload can leave an empty asset in the "starter"
        # state. It is a write that did not happen: the snapshot before it
        # is the record, so it is skipped (never read, never removed).
        ctx.note(f"Skipped incomplete record snapshots {sorted(incomplete)} (failed writes).")
    if not complete:
        ctx.note(f"The GitHub Release {ctx.tag} has no complete {RECORD_PREFIX}<n>.json: no upload was started.")
        return None
    name = record_asset(max(complete))
    with tempfile.TemporaryDirectory() as directory:
        download = gh("release", "download", ctx.tag, "--pattern", name, "--dir", directory)
        if download.returncode != 0:
            raise ReleaseError(f"Could not download {name} from {ctx.tag}: {download.stderr.strip()}")
        with open(os.path.join(directory, name), encoding="utf-8") as handle:
            try:
                record = json.load(handle)
            except json.JSONDecodeError as error:
                raise ReleaseError(f"{name} on {ctx.tag} is not valid JSON.") from error
    if not isinstance(record, dict) or record.get("version") != ctx.version or not isinstance(record.get("attempts"), list):
        raise ReleaseError(f"{name} on {ctx.tag} is not a record for {ctx.version}.")
    ctx.note(f"Deployment record {name}: {json.dumps(record['attempts'])}")
    return record


def write_record(ctx: Context, record: dict) -> None:
    """Adds the next snapshot; an existing asset is never replaced."""
    if ctx.read_only:
        raise ReleaseError("Internal error: a read-only run tried to write the deployment record.")
    name = record_asset(ctx.record_seq + 1)
    with tempfile.TemporaryDirectory() as directory:
        path = os.path.join(directory, name)
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(record, handle, indent=2)
            handle.write("\n")
        # No --clobber: it deletes the existing asset before the upload, and
        # an asset of this name already existing must fail the write.
        result = gh("release", "upload", ctx.tag, path)
    if result.returncode != 0:
        raise ReleaseError(f"Could not write {name} to {ctx.tag}: {result.stderr.strip()}")
    ctx.record_seq += 1


def failed_count(record: dict | None) -> int:
    return sum(1 for attempt in (record or {}).get("attempts", []) if attempt.get("state") == "failed")


def mark_failed(ctx: Context, record: dict, attempt: dict, reason: str) -> None:
    attempt["state"] = "failed"
    attempt["failure"] = reason
    if ctx.read_only:
        ctx.note("Read-only run: the failed attempt is not written to the record.")
    else:
        write_record(ctx, record)


# --- Decision ----------------------------------------------------------------


def decide(ctx: Context) -> str:
    statuses = central_gate(ctx)
    ctx.note(f"Maven Central: {gate_summary(statuses)}.")
    if all(status == 200 for status in statuses.values()):
        verify_central_aar(ctx)
        return "verified"

    record = read_record(ctx)
    attempts = (record or {}).get("attempts", [])
    if not attempts:
        return "upload"
    latest = attempts[-1]
    state = latest.get("state")
    if state == "failed":
        return upload_after_failures(record)
    deployment_id = latest.get("deploymentId")
    if not deployment_id:
        raise ReleaseError(
            f"An upload was started (run {latest.get('runId')}, attempt {latest.get('runAttempt')}) but its "
            "deployment ID was never recorded, so a deployment may exist. Failing closed: follow "
            "'Upload started but not recorded' in docs/releasing.md."
        )

    answer = wait_while_in_flight(ctx, deployment_id, {"PUBLISHED"})
    portal_state = answer["deploymentState"]
    ctx.note(f"Central Portal deployment {deployment_id}: {portal_state}.")
    if portal_state == "PUBLISHED":
        return wait_for_central(ctx)
    if portal_state == "FAILED":
        errors = json.dumps(answer.get("errors"), indent=2)
        ctx.note(f"The deployment failed validation:\n```\n{errors}\n```")
        mark_failed(ctx, record, latest, f"Portal FAILED: {errors}")
        return upload_after_failures(record)
    if portal_state == "VALIDATED":
        raise ReleaseError(
            f"Deployment {deployment_id} is VALIDATED and waits for a manual publish, which this pipeline "
            "never requests. Failing closed: check it in the Central Portal UI (see docs/releasing.md)."
        )
    if portal_state in IN_FLIGHT_STATES:
        raise ReleaseError(
            f"Deployment {deployment_id} is still {portal_state} after the wait. Re-run the failed job later; "
            "it resumes from the recorded deployment."
        )
    raise ReleaseError(f"Deployment {deployment_id} has the unknown state {portal_state}. Failing closed.")


def upload_after_failures(record: dict | None) -> str:
    failures = failed_count(record)
    if failures >= MAX_FAILED_ATTEMPTS:
        raise ReleaseError(
            f"{failures} deployments of this version have failed. The failure is not transient; "
            "fix it and release a new patch version (see docs/releasing.md)."
        )
    return "upload"


# --- Upload ------------------------------------------------------------------


def bundle_aar_sha256(ctx: Context, bundle: str) -> str:
    entry = f"{central_path(ctx.version)}/{ARTIFACT}-{ctx.version}.aar"
    with zipfile.ZipFile(bundle) as archive:
        try:
            return hashlib.sha256(archive.read(entry)).hexdigest()
        except KeyError as error:
            raise ReleaseError(f"The bundle {bundle} has no {entry}.") from error


def multipart(bundle: str) -> tuple[bytes, str]:
    boundary = uuid.uuid4().hex
    with open(bundle, "rb") as handle:
        content = handle.read()
    body = (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="bundle"; filename="{os.path.basename(bundle)}"\r\n'
        "Content-Type: application/octet-stream\r\n\r\n"
    ).encode() + content + f"\r\n--{boundary}--\r\n".encode()
    return body, f"multipart/form-data; boundary={boundary}"


def upload(ctx: Context, bundle: str) -> str:
    if ctx.read_only:
        raise ReleaseError("A read-only run never uploads.")
    actual = bundle_aar_sha256(ctx, bundle)
    if actual != ctx.expected_sha256:
        raise ReleaseError(f"The bundle's AAR has SHA-256 {actual}, expected {ctx.expected_sha256}.")
    decision = decide(ctx)
    if decision != "upload":
        ctx.note(f"Decision: {decision}; nothing to upload.")
        return decision

    # Everything that can fail locally happens before the marker, so a run
    # that never sent the bundle (a missing secret, an unreadable bundle)
    # leaves no upload-started attempt behind.
    portal_token()
    body, content_type = multipart(bundle)
    record = read_record(ctx) or {"version": ctx.version, "attempts": []}
    attempt = {
        "state": "upload-started",
        "aarSha256": actual,
        "bundleSha256": sha256_file(bundle),
        "runId": os.environ.get("GITHUB_RUN_ID", ""),
        "runAttempt": os.environ.get("GITHUB_RUN_ATTEMPT", ""),
        "time": now(),
    }
    record["attempts"].append(attempt)
    # Written before the upload: a run that dies after posting the bundle
    # leaves this attempt without an ID, and the next run fails closed
    # instead of uploading again.
    write_record(ctx, record)

    name = f"{GROUP}-{ARTIFACT}-{ctx.version}"
    status, payload = portal_request(
        ctx,
        "POST",
        f"/api/v1/publisher/upload?name={name}&publishingType=AUTOMATIC",
        body=body,
        content_type=content_type,
        timeout=env_int("UPLOAD_TIMEOUT", 600),
        retry=False,
    )
    text = payload.decode(errors="replace").strip()
    if status not in (200, 201):
        # A 4xx answer means the Portal refused the bundle; no deployment exists.
        mark_failed(ctx, record, attempt, f"Upload refused: HTTP {status}: {text[:500]}")
        raise ReleaseError(f"The Central Portal refused the upload: HTTP {status}: {text[:500]}")
    deployment_id = text
    ctx.note(f"Uploaded the bundle as Central Portal deployment {deployment_id}.")
    attempt["state"] = "uploaded"
    attempt["deploymentId"] = deployment_id
    write_record(ctx, record)

    answer = wait_while_in_flight(ctx, deployment_id, {"PUBLISHING", "PUBLISHED"})
    portal_state = answer["deploymentState"]
    ctx.note(f"Central Portal deployment {deployment_id}: {portal_state}.")
    if portal_state in ("PUBLISHING", "PUBLISHED"):
        return "uploaded"
    if portal_state == "FAILED":
        errors = json.dumps(answer.get("errors"), indent=2)
        ctx.note(f"The deployment failed validation:\n```\n{errors}\n```")
        mark_failed(ctx, record, attempt, f"Portal FAILED: {errors}")
        raise ReleaseError(f"Deployment {deployment_id} failed validation.")
    raise ReleaseError(
        f"Deployment {deployment_id} is still {portal_state} after the wait. Re-run the failed job later; "
        "it resumes from the recorded deployment."
    )


def probe(ctx: Context) -> str:
    status, payload = portal_request(ctx, "POST", f"/api/v1/publisher/status?id={PROBE_DEPLOYMENT_ID}")
    ctx.note(f"Central Portal status query for an unknown deployment: HTTP {status} {payload.decode(errors='replace')[:200]}")
    if status in (401, 403):
        raise ReleaseError(f"The Central Portal rejected the credentials (HTTP {status}).")
    if status not in (400, 404):
        raise ReleaseError(f"Unexpected HTTP {status} for an unknown deployment. Failing closed.")
    return "credentials-accepted"


# --- Entry point -------------------------------------------------------------


def write_outputs(ctx: Context, command: str, result: str | None, error: str | None) -> None:
    output = os.environ.get("GITHUB_OUTPUT")
    if output and result:
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(f"decision={result}\n")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        mode = " (read-only)" if ctx.read_only else ""
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(f"### Maven Central {command} for {ctx.version}{mode}\n\n")
            for line in ctx.evidence:
                handle.write(f"{line}\n\n")
            handle.write(f"**Result:** {result}\n\n" if result else f"**Failed:** {error}\n\n")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["decide", "upload", "verify", "probe"])
    parser.add_argument("version")
    parser.add_argument("--expected-sha256", default="", help="SHA-256 of the GitHub Release AAR")
    parser.add_argument("--bundle", help="the Central Portal bundle (upload)")
    parser.add_argument("--read-only", action="store_true")
    parser.add_argument("--no-record", action="store_true", help="no GitHub Release exists, so there is no record")
    args = parser.parse_args(argv)

    if args.command != "probe" and len(args.expected_sha256) != 64:
        parser.error("--expected-sha256 is required")
    if args.command == "upload" and not args.bundle:
        parser.error("upload needs --bundle")
    ctx = Context(args.version, args.expected_sha256.lower(), args.read_only, not args.no_record)
    try:
        if args.command == "decide":
            result = decide(ctx)
        elif args.command == "upload":
            result = upload(ctx, args.bundle)
        elif args.command == "verify":
            result = verify(ctx)
        else:
            result = probe(ctx)
    except ReleaseError as error:
        print(f"error: {error}", file=sys.stderr)
        write_outputs(ctx, args.command, None, str(error))
        return 1
    print(f"decision={result}")
    write_outputs(ctx, args.command, result, None)
    return 0


if __name__ == "__main__":
    sys.exit(main())
