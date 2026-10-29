#!/usr/bin/env python3
"""Release bookkeeping. No build or application dependencies are required."""

import argparse
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def git_file(commit, path):
    return subprocess.check_output(["git", "show", f"{commit}:{path}"], text=True)


def version_parts(version):
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("expected a stable X.Y.Z version")
    return tuple(map(int, version.split(".")))


def next_version(version, bump):
    major, minor, patch = version_parts(version)
    if bump == "major":
        return f"{major + 1}.0.0"
    if bump == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def package_version(manifest):
    package = tomllib.loads(manifest)["package"]
    if package["name"] != "tuivir":
        raise ValueError("expected the tuivir package")
    version_parts(package["version"])
    return package["version"]


def version_documents(manifest, lockfile, version):
    """Change only Tuivir's version lines, preserving dependency resolution."""
    old = package_version(manifest)
    packages = [p for p in tomllib.loads(lockfile)["package"]
                if p["name"] == "tuivir" and "source" not in p]
    if len(packages) != 1 or packages[0]["version"] != old:
        raise ValueError("Cargo.lock must contain the matching local tuivir version")

    def replace(block):
        return re.sub(r'(?m)^(version\s*=\s*")[^"]+(".*)$',
                      lambda match: match[1] + version + match[2], block, count=1)

    manifest = re.sub(r"(?ms)^\[package\]\n.*?(?=^\[|\Z)",
                      lambda match: replace(match[0]), manifest)
    lockfile = re.sub(
        r"(?ms)^\[\[package\]\]\n.*?(?=^\[\[package\]\]|\Z)",
        lambda match: replace(match[0])
        if tomllib.loads(match[0])["package"][0] == packages[0] else match[0],
        lockfile,
    )
    return manifest, lockfile


def merged_tag(event, repository):
    pr = event["pull_request"]
    if (not pr["merged"] or event["repository"]["full_name"] != repository
            or pr["base"]["repo"]["full_name"] != repository
            or pr["head"]["repo"]["full_name"] != repository
            or pr["base"]["ref"] != event["repository"]["default_branch"]):
        raise ValueError("expected a merged release PR from this repository into its default branch")
    commit = pr["merge_commit_sha"]
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("invalid merge commit")
    git("merge-base", "--is-ancestor", commit,
        f"refs/remotes/origin/{event['repository']['default_branch']}")
    version = package_version(git_file(commit, "Cargo.toml"))
    if pr["head"]["ref"] != f"release/v{version}":
        raise ValueError("release branch must match the merged package version")
    parent = git("rev-parse", f"{commit}^1")
    paths = ("Cargo.toml", "Cargo.lock")
    if set(git("diff", "--name-only", parent, commit).splitlines()) != set(paths):
        raise ValueError("release PR must change only Cargo.toml and Cargo.lock")
    previous = tuple(git_file(parent, path) for path in paths)
    old = package_version(previous[0])
    if version not in [next_version(old, kind) for kind in ("patch", "minor", "major")]:
        raise ValueError("release must be one patch, minor, or major bump")
    if tuple(git_file(commit, path) for path in paths) != version_documents(*previous, version):
        raise ValueError("release PR must change only the package version")
    return {"tag": f"v{version}", "version": version, "commit": commit}


def create_tag(tag, commit):
    if not tag.startswith("v"):
        raise ValueError("tag must start with v")
    version_parts(tag[1:])
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("expected a full commit SHA")
    if package_version(git_file(commit, "Cargo.toml")) != tag[1:]:
        raise ValueError("tag must match the package version at the release commit")
    ref = f"refs/tags/{tag}"
    remote = dict(line.split()[::-1] for line in
                  git("ls-remote", "--tags", "origin", ref, ref + "^{}").splitlines())
    if remote:
        if remote.get(ref + "^{}", remote.get(ref)) != commit:
            raise ValueError("existing remote tag points at a different commit; refusing to move it")
        return
    local = git("tag", "--list", tag)
    if local:
        if git("rev-parse", ref + "^{commit}") != commit:
            raise ValueError("existing local tag points at a different commit")
    else:
        git("tag", "-a", tag, commit, "-m", f"Release {tag}")
    git("push", "origin", ref)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    bump = commands.add_parser("bump", help="update Cargo files and print the next version")
    bump.add_argument("kind", choices=("patch", "minor", "major"))
    merged = commands.add_parser("merged-tag", help="validate a merged release PR event")
    merged.add_argument("event", type=Path)
    merged.add_argument("--repo", required=True)
    tag = commands.add_parser("tag", help="create an annotated tag; never replace an existing tag")
    tag.add_argument("name")
    tag.add_argument("commit")
    args = parser.parse_args()
    if args.command == "bump":
        manifest = Path("Cargo.toml").read_text()
        lockfile = Path("Cargo.lock").read_text()
        version = next_version(package_version(manifest), args.kind)
        manifest, lockfile = version_documents(manifest, lockfile, version)
        Path("Cargo.toml").write_text(manifest)
        Path("Cargo.lock").write_text(lockfile)
        print(version)
    elif args.command == "merged-tag":
        print(json.dumps(merged_tag(json.loads(args.event.read_text()), args.repo)))
    elif args.command == "tag":
        create_tag(args.name, args.commit)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, subprocess.CalledProcessError) as error:
        sys.exit(f"release: {error}")
