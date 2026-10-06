#!/usr/bin/env python3
"""Validate release inputs and publish only a complete, verified asset set."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def version(tag):
    if not re.fullmatch(r"v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", tag):
        raise ValueError("Expected a stable version tag vX.Y.Z (no leading zeros)")
    return tag[1:]


def check_versions(tag, read):
    expected = version(tag)
    for name in ["Cargo.toml", "desktop/src-tauri/Cargo.toml"]:
        if tomllib.loads(read(name))["package"]["version"] != expected:
            raise ValueError(f"Version mismatch: {name} must be {expected}")
    for name in ["desktop/package.json", "desktop/src-tauri/tauri.conf.json"]:
        if json.loads(read(name))["version"] != expected:
            raise ValueError(f"Version mismatch: {name} must be {expected}")
    lock = json.loads(read("desktop/package-lock.json"))
    if lock["version"] != expected or lock["packages"][""]["version"] != expected:
        raise ValueError("Version mismatch: desktop/package-lock.json")
    for name, packages in [
        ("Cargo.lock", ["macrun"]),
        ("desktop/src-tauri/Cargo.lock", ["macrun", "macrun-desktop"]),
    ]:
        entries = tomllib.loads(read(name))["package"]
        for package in packages:
            local = [p for p in entries if p["name"] == package and "source" not in p]
            if len(local) != 1 or local[0]["version"] != expected:
                raise ValueError(f"Version mismatch: {name}: {package}")
    return expected


def resolve(tag):
    version(tag)
    sha = run("git", "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}")
    remote = json.loads(
        run("gh", "api", f"repos/{{owner}}/{{repo}}/git/ref/tags/{tag}")
    )["object"]
    for _ in range(10):
        if remote["type"] != "tag":
            break
        remote = json.loads(
            run("gh", "api", f"repos/{{owner}}/{{repo}}/git/tags/{remote['sha']}")
        )["object"]
    if remote["type"] != "commit" or remote["sha"] != sha:
        raise ValueError("Remote tag no longer matches the checked-out tag")
    run("git", "merge-base", "--is-ancestor", sha, "origin/main")
    check_versions(tag, lambda name: run("git", "show", f"{sha}:{name}"))
    return sha


def asset_names(tag):
    v = version(tag)
    return {f"Macrun-Desktop_{v}_macos_{arch}.dmg" for arch in ["arm64", "x86_64"]} | {
        name
        for arch in ["amd64", "arm64"]
        for name in [f"macrun_{v}_{arch}.deb", f"macrun_{v}_linux_{arch}.tar.gz"]
    }


def checksum(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def collect(directory, tag, sha):
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Expected a full commit SHA")
    expected = asset_names(tag)
    files = {p.name for p in directory.iterdir()}
    if files != expected:
        raise ValueError(
            f"Asset set mismatch: missing={expected - files}, extra={files - expected}"
        )
    if any(
        not (directory / name).is_file()
        or (directory / name).is_symlink()
        or (directory / name).stat().st_size == 0
        for name in expected
    ):
        raise ValueError("Assets must be nonempty regular files")
    manifest = {
        "tag": tag,
        "commit": sha,
        "assets": {
            name: {
                "sha256": checksum(directory / name),
                "size": (directory / name).stat().st_size,
            }
            for name in sorted(expected)
        },
    }
    (directory / "release-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    (directory / "SHA256SUMS").write_text(
        "".join(
            f"{checksum(directory / name)}  {name}\n"
            for name in sorted(expected | {"release-manifest.json"})
        )
    )


def release_record(tag):
    # A failing API request must fail the run, never masquerade as "no release".
    records = json.loads(
        run(
            "gh",
            "api",
            "--paginate",
            "--slurp",
            "repos/{owner}/{repo}/releases?per_page=100",
        )
    )
    return next((r for page in records for r in page if r["tag_name"] == tag), None)


def assert_draft(record, sha):
    if record and (
        not record["draft"]
        or f"<!-- macrun-release:{sha} -->" not in (record["body"] or "")
    ):
        raise ValueError(
            "Refusing a published release or a draft owned by a different run/commit"
        )


def publish(directory, tag, sha, make_public=False):
    # Re-resolve immediately before writes; moved tags must not publish stale binaries.
    if resolve(tag) != sha:
        raise ValueError("Tag moved after build")
    manifest = json.loads((directory / "release-manifest.json").read_text())
    expected = asset_names(tag) | {"release-manifest.json", "SHA256SUMS"}
    if (
        manifest["tag"] != tag
        or manifest["commit"] != sha
        or set(manifest["assets"]) != asset_names(tag)
    ):
        raise ValueError("Manifest identity mismatch")
    if {p.name for p in directory.iterdir()} != expected:
        raise ValueError("Unexpected publish asset set")
    for name, item in manifest["assets"].items():
        if (
            checksum(directory / name) != item["sha256"]
            or (directory / name).stat().st_size != item["size"]
        ):
            raise ValueError(f"Asset changed: {name}")
    sums = "".join(
        f"{checksum(directory / name)}  {name}\n"
        for name in sorted(expected - {"SHA256SUMS"})
    )
    if (directory / "SHA256SUMS").read_text() != sums:
        raise ValueError("Checksum list changed")
    record = release_record(tag)
    assert_draft(record, sha)
    if record is None:
        with tempfile.TemporaryDirectory() as temp:
            notes = Path(temp) / "notes.md"
            notes.write_text(
                f"<!-- macrun-release:{sha} -->\n\n"
                "macOS: open the DMG and drag Macrun Desktop into Applications.\n"
                "Linux: install the matching .deb with apt, or unpack the tar.gz. "
                "See the bundled installation instructions.\n\n"
                f"Source commit: `{sha}`. Verify downloads with `SHA256SUMS`.\n"
            )
            run(
                "gh",
                "release",
                "create",
                tag,
                "--verify-tag",
                "--draft",
                "--title",
                tag,
                "--notes-file",
                str(notes),
            )
        record = release_record(tag)
    assert_draft(record, sha)
    if record is None:
        raise ValueError("Draft was not created")
    if {a["name"] for a in record["assets"]} - expected:
        raise ValueError(
            "Draft contains unexpected attachments; inspect it before retrying"
        )
    run(
        "gh",
        "release",
        "upload",
        tag,
        *[str(directory / name) for name in sorted(expected)],
        "--clobber",
    )
    record = release_record(tag)
    assert_draft(record, sha)
    if record is None or {a["name"] for a in record["assets"]} != expected:
        raise ValueError("Uploaded attachment set is incomplete")
    with tempfile.TemporaryDirectory() as temp:
        run("gh", "release", "download", tag, "--dir", temp)
        for name in expected:
            if checksum(Path(temp) / name) != checksum(directory / name):
                raise ValueError(f"Uploaded asset checksum mismatch: {name}")
    if make_public:
        if resolve(tag) != sha:
            raise ValueError("Tag moved before publication")
        assert_draft(release_record(tag), sha)
        run("gh", "release", "edit", tag, "--draft=false")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["resolve", "check", "collect", "publish"])
    parser.add_argument("--tag", required=True)
    parser.add_argument("--sha")
    parser.add_argument("--directory", type=Path, default=Path("dist/release"))
    parser.add_argument("--public", action="store_true")
    args = parser.parse_args()
    if args.command == "resolve":
        sha = resolve(args.tag)
        assert_draft(release_record(args.tag), sha)
        with open(os.environ["GITHUB_OUTPUT"], "a") as output:
            output.write(f"sha={sha}\nversion={version(args.tag)}\n")
    elif args.command == "check":
        check_versions(args.tag, lambda name: Path(name).read_text())
    elif args.command == "collect":
        collect(args.directory, args.tag, args.sha or "")
    else:
        publish(args.directory, args.tag, args.sha or "", args.public)


if __name__ == "__main__":
    main()
