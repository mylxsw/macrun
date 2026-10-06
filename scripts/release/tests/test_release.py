import contextlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location(
    "release", ROOT / "scripts/release/release.py"
)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
SHA = "a" * 40
TAG = "v0.2.0"


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        for name in release.asset_names(TAG):
            (self.directory / name).write_bytes(name.encode())

    def collected(self):
        release.collect(self.directory, TAG, SHA)

    def record(self, names=()):
        return {
            "tag_name": TAG,
            "draft": True,
            "body": f"<!-- macrun-release:{SHA} -->",
            "assets": [{"name": n} for n in names],
        }

    def test_version_validation(self):
        self.assertEqual(release.version(TAG), "0.2.0")
        for bad in [
            "0.2.0",
            "v01.2.0",
            "v1.2.3-rc.1",
            "../../x",
            "v1.2.3\n",
            "v1.2.3;echo x",
        ]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                release.version(bad)

    def test_repository_versions_and_every_mismatch(self):
        self.assertEqual(
            release.check_versions(TAG, lambda n: (ROOT / n).read_text()), "0.2.0"
        )
        names = [
            "Cargo.toml",
            "desktop/src-tauri/Cargo.toml",
            "desktop/package.json",
            "desktop/src-tauri/tauri.conf.json",
            "desktop/package-lock.json",
            "Cargo.lock",
            "desktop/src-tauri/Cargo.lock",
        ]
        for name in names:
            with self.subTest(name=name), self.assertRaises(ValueError):
                release.check_versions(
                    TAG,
                    lambda n: (ROOT / n).read_text().replace("0.2.0", "9.9.9")
                    if n == name
                    else (ROOT / n).read_text(),
                )

    def test_resolve_annotated_tag(self):
        def run(*args):
            if args[:2] == ("git", "rev-parse"):
                return SHA
            if args[:2] == ("git", "merge-base"):
                return ""
            if args[:2] == ("git", "show"):
                return (ROOT / args[2].split(":", 1)[1]).read_text()
            if "/git/ref/" in args[2]:
                return json.dumps({"object": {"type": "tag", "sha": "b" * 40}})
            return json.dumps({"object": {"type": "commit", "sha": SHA}})

        with patch.object(release, "run", side_effect=run):
            self.assertEqual(release.resolve(TAG), SHA)

    def test_moved_remote_tag(self):
        with (
            patch.object(
                release,
                "run",
                side_effect=[
                    SHA,
                    json.dumps({"object": {"type": "commit", "sha": "b" * 40}}),
                ],
            ),
            self.assertRaisesRegex(ValueError, "Remote tag"),
        ):
            release.resolve(TAG)

    def test_release_lookup_handles_pages_and_failure(self):
        record = self.record()
        with patch.object(release, "run", return_value=json.dumps([[], [record]])):
            self.assertEqual(release.release_record(TAG), record)
            self.assertIsNone(release.release_record("v9.0.0"))
        with (
            patch.object(
                release, "run", side_effect=subprocess.CalledProcessError(1, "gh")
            ),
            self.assertRaises(subprocess.CalledProcessError),
        ):
            release.release_record(TAG)

    def test_reject_published_or_unowned_draft(self):
        for record in [
            {**self.record(), "draft": False},
            {**self.record(), "body": None},
            {**self.record(), "body": "user notes"},
        ]:
            with self.assertRaises(ValueError):
                release.assert_draft(record, SHA)

    def test_exact_manifest_and_checksum_contents(self):
        self.collected()
        manifest = json.loads((self.directory / "release-manifest.json").read_text())
        self.assertEqual(manifest["commit"], SHA)
        self.assertEqual(set(manifest["assets"]), release.asset_names(TAG))
        lines = (self.directory / "SHA256SUMS").read_text().splitlines()
        self.assertEqual(len(lines), 7)
        for line in lines:
            digest, name = line.split("  ")
            self.assertEqual(digest, release.checksum(self.directory / name))

    def test_collect_rejects_incomplete_extra_empty_and_symlink(self):
        name = next(iter(release.asset_names(TAG)))
        path = self.directory / name
        original = path.read_bytes()
        path.unlink()
        with self.assertRaises(ValueError):
            self.collected()
        path.write_bytes(b"")
        with self.assertRaises(ValueError):
            self.collected()
        path.unlink()
        path.symlink_to(next(p for p in self.directory.iterdir()))
        with self.assertRaises(ValueError):
            self.collected()
        path.unlink()
        path.write_bytes(original)
        (self.directory / "extra").write_text("x")
        with self.assertRaises(ValueError):
            self.collected()
        with self.assertRaises(ValueError):
            release.collect(self.directory, TAG, "short")

    def test_publish_stops_on_local_corruption(self):
        self.collected()
        cases = [
            "release-manifest.json",
            "SHA256SUMS",
            next(iter(release.asset_names(TAG))),
            "extra",
        ]
        for name in cases:
            path = self.directory / name
            original = path.read_bytes() if path.exists() else None
            if name == "release-manifest.json":
                data = json.loads(path.read_text())
                data["commit"] = "b" * 40
                path.write_text(json.dumps(data))
            else:
                path.write_text("corrupted")
            with (
                self.subTest(name=name),
                patch.object(release, "resolve", return_value=SHA),
                patch.object(release, "run") as run,
                self.assertRaises(ValueError),
            ):
                release.publish(self.directory, TAG, SHA, True)
            run.assert_not_called()
            if original is None:
                path.unlink()
            else:
                path.write_bytes(original)

    def publish_mock(self, public, records, corrupt_remote=False, fail_upload=False):
        calls = []

        def run(*args):
            calls.append(args)
            if args[:3] == ("gh", "release", "download"):
                dest = Path(args[-1])
                for path in self.directory.iterdir():
                    shutil.copyfile(path, dest / path.name)
                if corrupt_remote:
                    (dest / "SHA256SUMS").write_text("wrong")
            if args[:3] == ("gh", "release", "upload") and fail_upload:
                raise subprocess.CalledProcessError(1, args)
            return ""

        with (
            patch.object(release, "resolve", return_value=SHA),
            patch.object(release, "release_record", side_effect=records),
            patch.object(release, "run", side_effect=run),
        ):
            release.publish(self.directory, TAG, SHA, public)
        return calls

    def test_new_draft_and_verified_publication(self):
        self.collected()
        complete = self.record(p.name for p in self.directory.iterdir())
        calls = self.publish_mock(True, [None, self.record(), complete, complete])
        self.assertEqual(calls[-1], ("gh", "release", "edit", TAG, "--draft=false"))
        self.assertEqual(calls[0][:3], ("gh", "release", "create"))

    def test_draft_mode_and_partial_upload_retry(self):
        self.collected()
        complete = self.record(p.name for p in self.directory.iterdir())
        calls = self.publish_mock(False, [self.record(["SHA256SUMS"]), complete])
        self.assertFalse(any(c[2] in ["create", "edit"] for c in calls))

    def test_publish_refuses_bad_remote_states(self):
        self.collected()
        complete = self.record(p.name for p in self.directory.iterdir())
        cases = [
            ([None, None], False),
            ([self.record(["foreign.txt"])], False),
            ([self.record(), self.record()], False),
            ([self.record(), complete], True),
        ]
        for records, corruption in cases:
            with (
                self.subTest(records=records),
                self.assertRaises((ValueError, FileNotFoundError)),
            ):
                self.publish_mock(True, records, corrupt_remote=corruption)
        with self.assertRaises(subprocess.CalledProcessError):
            self.publish_mock(True, [self.record()], fail_upload=True)

    def test_tag_moved_stops_before_writes(self):
        self.collected()
        with (
            patch.object(release, "resolve", return_value="b" * 40),
            patch.object(release, "run") as run,
            self.assertRaisesRegex(ValueError, "Tag moved"),
        ):
            release.publish(self.directory, TAG, SHA, True)
        run.assert_not_called()

    def test_cli_check_collect_resolve_and_publish_dispatch(self):
        with (
            contextlib.chdir(ROOT),
            patch.object(sys, "argv", ["release.py", "check", "--tag", TAG]),
        ):
            release.main()
        with patch.object(
            sys,
            "argv",
            [
                "release.py",
                "collect",
                "--tag",
                TAG,
                "--sha",
                SHA,
                "--directory",
                str(self.directory),
            ],
        ):
            release.main()
        output = self.directory / "output"
        with (
            patch.dict(os.environ, {"GITHUB_OUTPUT": str(output)}),
            patch.object(release, "resolve", return_value=SHA),
            patch.object(release, "release_record", return_value=None),
            patch.object(sys, "argv", ["release.py", "resolve", "--tag", TAG]),
        ):
            release.main()
        self.assertIn(f"sha={SHA}", output.read_text())
        with (
            patch.object(release, "publish") as publish,
            patch.object(
                sys,
                "argv",
                ["release.py", "publish", "--tag", TAG, "--sha", SHA, "--public"],
            ),
        ):
            release.main()
        publish.assert_called_once_with(Path("dist/release"), TAG, SHA, True)


if __name__ == "__main__":
    unittest.main()
