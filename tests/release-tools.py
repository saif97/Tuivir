#!/usr/bin/env python3
"""Release command tests use only disposable files and Git repositories."""
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin")


class ReleaseTools(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "Cargo.toml").write_text('[package]\nname = "tuivir"\nversion = "0.2.7"\n\n[dependencies]\nexample = "0.2.7"\n')
        (self.root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "tuivir"\nversion = "0.2.7"\n')

    def command(self, *args, success=True):
        result = subprocess.run(args, cwd=self.root, text=True, capture_output=True)
        self.assertEqual(result.returncode == 0, success, result.stderr)
        return result.stdout.strip()

    def test_minor_bump_changes_both_versions_and_preserves_dependencies(self):
        self.assertEqual(self.command("python3", str(SCRIPTS / "bump-version.py"), "minor"), "0.3.0")
        manifest = tomllib.loads((self.root / "Cargo.toml").read_text())
        lockfile = tomllib.loads((self.root / "Cargo.lock").read_text())
        self.assertEqual(manifest["package"]["version"], "0.3.0")
        self.assertEqual(manifest["dependencies"], {"example": "0.2.7"})
        self.assertEqual(lockfile["package"][0]["version"], "0.3.0")

    def test_patch_and_major_bumps(self):
        files = {name: (self.root / name).read_text() for name in ("Cargo.toml", "Cargo.lock")}
        for kind, expected in (("patch", "0.2.8"), ("major", "1.0.0")):
            with self.subTest(kind=kind):
                for name, text in files.items():
                    (self.root / name).write_text(text)
                self.assertEqual(self.command("python3", str(SCRIPTS / "bump-version.py"), kind), expected)

    def test_bad_bump_or_mismatched_lock_does_not_change_the_manifest(self):
        path = self.root / "Cargo.toml"
        before = path.read_text()
        self.command("python3", str(SCRIPTS / "bump-version.py"), "automatic", success=False)
        (self.root / "Cargo.lock").write_text('version = 4\n')
        self.command("python3", str(SCRIPTS / "bump-version.py"), "minor", success=False)
        self.assertEqual(path.read_text(), before)

    def test_tag_retry_preserves_the_exact_commit_and_rejects_replacement(self):
        remote = tempfile.TemporaryDirectory()
        self.addCleanup(remote.cleanup)
        self.command("git", "init", "--bare", remote.name)
        self.command("git", "init", "-b", "master")
        self.command("git", "config", "user.name", "Test")
        self.command("git", "config", "user.email", "test@example.invalid")
        self.command("git", "add", ".")
        self.command("git", "commit", "-m", "Previous source")
        before = self.command("git", "rev-parse", "HEAD")
        self.command("python3", str(SCRIPTS / "bump-version.py"), "minor")
        self.command("git", "commit", "-am", "Release source")
        commit = self.command("git", "rev-parse", "HEAD")
        self.command("git", "remote", "add", "origin", remote.name)
        self.command("git", "commit", "--allow-empty", "-m", "Later source")
        self.assertEqual(self.command("bash", str(SCRIPTS / "tag-release"), before, commit), "v0.3.0")
        first = self.command("git", "ls-remote", "--tags", "origin")
        self.command("bash", str(SCRIPTS / "tag-release"), before, commit)
        self.command("bash", str(SCRIPTS / "tag-release"), before, "HEAD", success=False)
        self.assertEqual(self.command("bash", str(SCRIPTS / "tag-release"), commit, "HEAD"), "")
        self.assertEqual(self.command("git", "ls-remote", "--tags", "origin"), first)
        self.assertIn(commit + "\trefs/tags/v0.3.0^{}", first)

    def formula(self):
        path = self.root / "tuivir.rb"
        text = '  version "0.2.0"\n'
        for target in TARGETS:
            text += f'      url "https://github.com/saif97/Tuivir/releases/download/v0.2.0/tuivir-v0.2.0-{target}.tar.gz"\n      sha256 "' + "0" * 64 + '"\n'
        path.write_text(text + '    assert_equal "0.2.0\\n", shell_output("#{bin}/tuivir --version")\n')
        sums = self.root / "SHA256SUMS"
        sums.write_text("\n".join(digest * 64 + f"  tuivir-v0.3.0-{target}.tar.gz" for digest, target in zip("ab", TARGETS)))
        return path, sums

    def test_formula_updates_both_architectures_and_version_assertion(self):
        path, sums = self.formula()
        self.command("python3", str(SCRIPTS / "update-homebrew.py"), "v0.3.0", str(sums), str(path))
        result = path.read_text()
        self.assertNotIn("0.2.0", result)
        self.assertIn('assert_equal "0.3.0\\n"', result)
        for digest, target in zip("ab", TARGETS):
            self.assertIn(f'tuivir-v0.3.0-{target}.tar.gz"\n      sha256 "' + digest * 64, result)
        self.command("python3", str(SCRIPTS / "update-homebrew.py"), "v0.3.0", str(sums), str(path))
        self.assertEqual(path.read_text(), result)

    def test_formula_rejects_missing_hashes_downgrades_and_changed_retries(self):
        path, sums = self.formula()
        before = path.read_text()
        valid_sums = sums.read_text()
        sums.write_text("")
        self.command("python3", str(SCRIPTS / "update-homebrew.py"), "v0.3.0", str(sums), str(path), success=False)
        self.assertEqual(path.read_text(), before)
        sums.write_text(valid_sums)
        self.command("python3", str(SCRIPTS / "update-homebrew.py"), "v0.1.0", str(sums), str(path), success=False)
        self.command("python3", str(SCRIPTS / "update-homebrew.py"), "v0.3.0", str(sums), str(path))
        before = path.read_text()
        sums.write_text(valid_sums.replace("a" * 64, "c" * 64))
        self.command("python3", str(SCRIPTS / "update-homebrew.py"), "v0.3.0", str(sums), str(path), success=False)
        self.assertEqual(path.read_text(), before)


if __name__ == "__main__":
    unittest.main()
