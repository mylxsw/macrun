"""Exercise orchestration with fake Apple tools; live notarization requires secrets."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]


class MacSigningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in [
            "scripts/release/macos.sh",
            "scripts/release/release.py",
            "Cargo.toml",
            "Cargo.lock",
            "desktop/src-tauri/Cargo.toml",
            "desktop/src-tauri/Cargo.lock",
            "desktop/package.json",
            "desktop/package-lock.json",
            "desktop/src-tauri/tauri.conf.json",
        ]:
            dest = self.root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, dest)
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        stub = bin_dir / "stub"
        stub.write_text(
            f"#!{sys.executable}\n"
            + """
import json, os, pathlib, shutil, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
root = pathlib.Path(os.environ["TEST_ROOT"])
with (root / "commands").open("a") as f: f.write(json.dumps([name, *args]) + "\\n")
if name == "npm":
    target = args[args.index("--target") + 1]
    app = root / f"desktop/src-tauri/target/{target}/release/bundle/macos/Macrun Desktop.app/Contents/MacOS"
    app.mkdir(parents=True)
    for binary in ["macrun", "macrun-desktop"]: (app / binary).write_text("binary")
elif name == "lipo": print(os.environ.get("TEST_ARCH", "arm64"))
elif name == "xcrun" and args[:2] == ["notarytool", "submit"]:
    print(json.dumps({"id": "test-submission", "status": os.environ.get("TEST_NOTARY", "Accepted")}))
elif name == "hdiutil" and args[0] == "create": pathlib.Path(args[-1]).write_text("dmg")
elif name == "ditto":
    if "-k" in args: pathlib.Path(args[-1]).write_text("zip")
    else: shutil.copytree(args[-2], args[-1])
"""
        )
        stub.chmod(0o755)
        for name in [
            "npm",
            "security",
            "lipo",
            "codesign",
            "spctl",
            "xcrun",
            "hdiutil",
            "ditto",
        ]:
            (bin_dir / name).symlink_to(stub)
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "TEST_ROOT": str(self.root),
            "APPLE_CERTIFICATE": "ZmFrZQ==",
            "APPLE_CERTIFICATE_PASSWORD": "test-password",
            "APPLE_SIGNING_IDENTITY": "Developer ID Application: Test (TESTTEAM)",
            "APPLE_API_ISSUER": "test-issuer",
            "APPLE_API_KEY": "test-key",
            "APPLE_API_PRIVATE_KEY": "test-private-key",
        }

    def invoke(self, **overrides):
        return subprocess.run(
            [
                "bash",
                str(self.root / "scripts/release/macos.sh"),
                "0.2.0",
                "aarch64-apple-darwin",
            ],
            env={**self.env, **overrides},
            text=True,
            capture_output=True,
        )

    def commands(self):
        path = self.root / "commands"
        return (
            [json.loads(line) for line in path.read_text().splitlines()]
            if path.exists()
            else []
        )

    def test_app_and_dmg_are_notarized_and_verified_before_completion(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        app = (
            self.root
            / "desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Macrun Desktop.app"
        )
        self.assertEqual(app.stat().st_mode & 0o555, 0o555)
        calls = self.commands()
        submits = [c for c in calls if c[:3] == ["xcrun", "notarytool", "submit"]]
        self.assertEqual(len(submits), 2)
        self.assertTrue(submits[0][3].endswith("app.zip"))
        self.assertTrue(submits[1][3].endswith(".dmg"))
        self.assertTrue(
            any(
                c[:3] == ["xcrun", "stapler", "validate"] and "/mount/" in c[-1]
                for c in calls
            )
        )
        self.assertTrue(any(c[:2] == ["security", "delete-keychain"] for c in calls))
        self.assertNotIn("test-private-key", result.stdout + result.stderr)

    def test_rejected_notarization_stops_dmg_and_cleans_keychain(self):
        result = self.invoke(TEST_NOTARY="Invalid")
        self.assertNotEqual(result.returncode, 0)
        calls = self.commands()
        self.assertFalse(any(c[:2] == ["hdiutil", "create"] for c in calls))
        self.assertTrue(any(c[:3] == ["xcrun", "notarytool", "log"] for c in calls))
        self.assertTrue(any(c[:2] == ["security", "delete-keychain"] for c in calls))

    def test_wrong_worker_architecture_never_reaches_notary(self):
        result = self.invoke(TEST_ARCH="x86_64")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(c[:2] == ["xcrun", "notarytool"] for c in self.commands()))
        self.assertTrue(
            any(c[:2] == ["security", "delete-keychain"] for c in self.commands())
        )

    def test_missing_secret_fails_before_keychain_mutation(self):
        result = self.invoke(APPLE_API_PRIVATE_KEY="")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Missing secret: APPLE_API_PRIVATE_KEY", result.stderr)
        self.assertEqual(self.commands(), [])


if __name__ == "__main__":
    unittest.main()
