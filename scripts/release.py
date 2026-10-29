#!/usr/bin/env python3
"""Release bookkeeping. No build or application dependencies are required."""

import argparse
import base64
import hashlib
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path


UPSTREAM = "saif97/Tuivir"
TAP = "saif97/homebrew-tap"
TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-musl")


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


def verified_archives(tag, directory):
    if not tag.startswith("v"):
        raise ValueError("expected a vX.Y.Z tag")
    version_parts(tag[1:])
    expected = {f"tuivir-{tag}-{target}.tar.gz" for target in TARGETS}
    checksums = {}
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        digest, filename = line.split()
        if filename in checksums or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ValueError("invalid or duplicate checksum entry")
        checksums[filename] = digest
    if set(checksums) != expected:
        raise ValueError("checksum manifest must contain exactly the three release archives")
    for filename, digest in checksums.items():
        if hashlib.sha256((directory / filename).read_bytes()).hexdigest() != digest:
            raise ValueError(f"checksum mismatch: {filename}")
    return checksums


def formula_text(version, arm_sha, intel_sha):
    return f'''class Tuivir < Formula
  desc "Inspect and operate local virtualization and container providers"
  homepage "https://github.com/{UPSTREAM}"
  version "{version}"
  license "MIT OR Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/{UPSTREAM}/releases/download/v{version}/tuivir-v{version}-aarch64-apple-darwin.tar.gz"
      sha256 "{arm_sha}"
    end

    on_intel do
      url "https://github.com/{UPSTREAM}/releases/download/v{version}/tuivir-v{version}-x86_64-apple-darwin.tar.gz"
      sha256 "{intel_sha}"
    end
  end

  def install
    bin.install "tuivir"
  end

  test do
    assert_equal "{version}\\n", shell_output("#{{bin}}/tuivir --version")
  end
end
'''


def api(endpoint, data=None, method="GET", missing=False, paginate=False):
    command = ["gh", "api", endpoint, "--method", method]
    if paginate:
        command += ["--paginate", "--slurp"]
    if data is not None:
        command += ["--input", "-"]
    result = subprocess.run(command, input=json.dumps(data) if data is not None else None,
                            text=True, capture_output=True)
    if result.returncode:
        if missing and "(HTTP 404)" in result.stderr:
            return None
        raise ValueError(result.stderr.strip())
    return json.loads(result.stdout)


def formula_version(content):
    versions = re.findall(r'^  version "([^"]+)"$', content, re.MULTILINE)
    hashes = re.findall(r'^      sha256 "([0-9a-f]{64})"$', content, re.MULTILINE)
    if len(versions) != 1 or len(hashes) != 2:
        raise ValueError("unexpected formula structure")
    version_parts(versions[0])
    if content != formula_text(versions[0], *hashes):
        raise ValueError("formula differs from the generated template; refusing to overwrite edits")
    return versions[0]


def update_tap(candidate):
    version = formula_version(candidate)
    endpoint = f"repos/{TAP}/contents/Formula/tuivir.rb"
    current = api(endpoint)
    content = base64.b64decode(current["content"]).decode()
    old = formula_version(content)
    if candidate == content:
        print("Homebrew formula is already up to date")
        return
    if version_parts(version) <= version_parts(old):
        raise ValueError("refusing to downgrade or replace the contents of an existing formula version")
    api(endpoint, {"message": f"Update Tuivir to {version}", "sha": current["sha"],
                   "content": base64.b64encode(candidate.encode()).decode()}, method="PUT")
    print(f"Updated Homebrew formula to {version}")


def release_record(tag):
    if not tag.startswith("v"):
        raise ValueError("expected a vX.Y.Z tag")
    version_parts(tag[1:])
    release = api(f"repos/{UPSTREAM}/releases/tags/{tag}", missing=True)
    if release is None:
        # The tag endpoint cannot find drafts. List releases using the same
        # authenticated token, retaining failures instead of treating them as 404.
        pages = api(f"repos/{UPSTREAM}/releases?per_page=100", paginate=True)
        matches = [item for page in pages for item in page if item["tag_name"] == tag]
        if len(matches) > 1:
            raise ValueError("multiple releases have this tag; inspect the drafts before retrying")
        release = matches[0] if matches else None
    if release and (release["tag_name"] != tag or release["prerelease"]):
        raise ValueError("expected the matching stable release")
    return release


def require_immutable(release):
    if not release.get("immutable"):
        raise ValueError("release is not immutable; enable repository release immutability before the next release")


def publish(tag, directory):
    sums = verified_archives(tag, directory)
    sums["SHA256SUMS"] = hashlib.sha256((directory / "SHA256SUMS").read_bytes()).hexdigest()
    expected = {name: "sha256:" + digest for name, digest in sums.items()}
    release = release_record(tag)
    if release and not release["draft"]:
        require_immutable(release)
        if {asset["name"]: asset["digest"] for asset in release["assets"]} != expected:
            raise ValueError("published assets differ; refusing to replace them")
        print("Release is already published with the expected assets")
        return
    if release and any(asset["name"] not in expected for asset in release["assets"]):
        raise ValueError("draft contains unexpected assets; inspect it before retrying")
    if not release:
        subprocess.run(["gh", "release", "create", tag, "--repo", UPSTREAM,
                        "--draft", "--verify-tag", "--generate-notes", "--title", f"Tuivir {tag}"], check=True)
    # Drafts are unpublished: a failed build/upload may safely be replaced here.
    # Published assets are never uploaded again; immutability also blocks races.
    subprocess.run(["gh", "release", "upload", tag,
                    *[str(directory / name) for name in sorted(expected)],
                    "--repo", UPSTREAM, "--clobber"], check=True)
    uploaded = release_record(tag)
    if {asset["name"]: asset["digest"] for asset in uploaded["assets"]} != expected:
        raise ValueError("uploaded assets do not match the verified artifacts")
    latest = api(f"repos/{UPSTREAM}/releases/latest", missing=True)
    make_latest = not latest or version_parts(tag[1:]) > version_parts(latest["tag_name"][1:])
    subprocess.run(["gh", "release", "edit", tag, "--repo", UPSTREAM,
                    "--draft=false", f"--latest={str(make_latest).lower()}"], check=True)
    require_immutable(release_record(tag))


def download(tag, directory):
    release = release_record(tag)
    if not release or release["draft"]:
        raise ValueError("release is not published")
    require_immutable(release)
    subprocess.run(["gh", "release", "download", tag, "--repo", UPSTREAM,
                    "--dir", str(directory), "--pattern", f"tuivir-{tag}-*.tar.gz",
                    "--pattern", "SHA256SUMS"], check=True)
    sums = verified_archives(tag, directory)
    sums["SHA256SUMS"] = hashlib.sha256((directory / "SHA256SUMS").read_bytes()).hexdigest()
    if {a["name"]: a["digest"] for a in release["assets"]} != {
            name: "sha256:" + digest for name, digest in sums.items()}:
        raise ValueError("downloaded files do not match the published asset digests")


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
    formula = commands.add_parser("formula", help="verify archives and generate the Homebrew formula")
    formula.add_argument("tag")
    formula.add_argument("artifacts", type=Path)
    formula.add_argument("output", type=Path)
    update = commands.add_parser("update-tap", help="commit a checked formula; reject downgrades and changed retries")
    update.add_argument("formula", type=Path)
    publish_parser = commands.add_parser("publish", help="finish a draft without replacing published assets")
    publish_parser.add_argument("tag")
    publish_parser.add_argument("artifacts", type=Path)
    published = commands.add_parser("published", help="report whether an immutable release already exists")
    published.add_argument("tag")
    download_parser = commands.add_parser("download", help="fetch and verify an immutable release for Homebrew")
    download_parser.add_argument("tag")
    download_parser.add_argument("directory", type=Path)
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
    elif args.command == "formula":
        sums = verified_archives(args.tag, args.artifacts)
        args.output.write_text(formula_text(args.tag[1:], *[
            sums[f"tuivir-{args.tag}-{target}.tar.gz"] for target in TARGETS[:2]]))
    elif args.command == "update-tap":
        update_tap(args.formula.read_text())
    elif args.command == "publish":
        publish(args.tag, args.artifacts)
    elif args.command == "published":
        release = release_record(args.tag)
        exists = bool(release and not release["draft"])
        if exists:
            require_immutable(release)
        print(str(exists).lower())
    elif args.command == "download":
        download(args.tag, args.directory)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"release: {error}")
