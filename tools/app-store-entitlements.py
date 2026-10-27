#!/usr/bin/env python3
"""Validate local Mac App Store signing inputs and materialize entitlements."""

import argparse
import datetime
import hashlib
import plistlib
import re
import subprocess
from pathlib import Path


def signing_identity(requested, policy, prefixes):
    output = subprocess.check_output(
        ["security", "find-identity", "-v", "-p", policy], text=True
    )
    identities = re.findall(r'\d+\) ([0-9A-Fa-f]{40}) "([^"]+)"', output)
    matches = [
        (fingerprint.upper(), name)
        for fingerprint, name in identities
        if requested == name or requested.upper() == fingerprint.upper()
    ]
    if len(matches) != 1 or not matches[0][1].startswith(prefixes):
        raise ValueError(f"No unique valid App Store signing identity for {requested!r}")
    return matches[0]


def store_entitlements(profile, base, bundle_id, app_fingerprint, installer_name):
    if "OSX" not in profile.get("Platform", []):
        raise ValueError("Provisioning profile must target macOS (OSX)")
    expiration = profile.get("ExpirationDate")
    if not isinstance(expiration, datetime.datetime) or expiration.replace(
        tzinfo=datetime.timezone.utc
    ) <= datetime.datetime.now(datetime.timezone.utc):
        raise ValueError("Provisioning profile is expired or has no expiration date")
    allowed = profile.get("Entitlements", {})
    if (
        "ProvisionedDevices" in profile
        or profile.get("ProvisionsAllDevices")
        or allowed.get("get-task-allow")
        or allowed.get("com.apple.security.get-task-allow")
    ):
        raise ValueError("Use a Mac App Store Connect distribution profile")
    teams = profile.get("TeamIdentifier", [])
    prefixes = profile.get("ApplicationIdentifierPrefix", [])
    if len(teams) != 1 or len(prefixes) != 1:
        raise ValueError("Profile must identify one team and one App ID prefix")
    app_id = f"{prefixes[0]}.{bundle_id}"
    if allowed.get("com.apple.application-identifier") != app_id:
        raise ValueError(f"Profile must explicitly authorize {app_id}")
    if allowed.get("com.apple.developer.team-identifier") != teams[0]:
        raise ValueError("Profile team entitlement does not match its team")
    fingerprints = {
        hashlib.sha1(certificate).hexdigest().upper()
        for certificate in profile.get("DeveloperCertificates", [])
    }
    if app_fingerprint not in fingerprints:
        raise ValueError("App signing certificate is not included in this profile")
    if not installer_name.endswith(f"({teams[0]})"):
        raise ValueError("Installer signing identity belongs to a different team")
    result = dict(base)
    result["com.apple.application-identifier"] = app_id
    result["com.apple.developer.team-identifier"] = teams[0]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True, type=Path)
    parser.add_argument("--template", required=True, type=Path)
    parser.add_argument("--bundle-id", required=True)
    parser.add_argument("--app-identity", required=True)
    parser.add_argument("--installer-identity", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    profile = plistlib.loads(
        subprocess.check_output(["security", "cms", "-D", "-i", str(args.profile)])
    )
    app_fingerprint, _ = signing_identity(
        args.app_identity,
        "codesigning",
        ("Apple Distribution:", "3rd Party Mac Developer Application:"),
    )
    _, installer_name = signing_identity(
        args.installer_identity, "basic", ("3rd Party Mac Developer Installer:",)
    )
    entitlements = store_entitlements(
        profile,
        plistlib.loads(args.template.read_bytes()),
        args.bundle_id,
        app_fingerprint,
        installer_name,
    )
    args.output.write_bytes(plistlib.dumps(entitlements))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"App Store signing preflight failed: {error}") from error
