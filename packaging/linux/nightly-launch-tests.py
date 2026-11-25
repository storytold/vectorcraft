#!/usr/bin/env python3
"""Synthetic launcher tests: every VectorCraft Nightly placement starts the GUI the flavor's way.

No build and no window: a recorder script stands in for the GUI, so the argv and environment the
launcher passes are read directly.
"""
import hashlib
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
LAUNCHER = HERE / "nightly-launch.sh"
# The plain tar's entrypoint is craft-common-actions' template rendered for this app; these are its bytes.
TAR_LAUNCHER = HERE / "nightly-tar-launch.sh"
TAR_PROFILE = "ai.storyteller.vectorcraft.nightly"
TAR_LAUNCHER_SHA256 = "c9046d6336057485b3c1346ba489fac957d02cfff8899f6b2cb501b5d780c1a9"

RECORDER = """#!/bin/sh
printf 'config=<%s>\n' "${VECTORCRAFT_CONFIG_DIR:-}"
for arg in "$@"; do
  printf '<%s>\n' "$arg"
done
"""

# Where each format puts the launcher and the GUI beside it: the plain tar, the deb/rpm, the
# AppImage's AppRun, the Arch handoff and the Flatpak (see package.sh, nightly-arch-tar.sh and
# flatpak-bundle.sh).
PLACEMENTS = {
    "tar": ("vectorcraft", "vectorcraft-bin"),
    "deb-rpm": ("usr/bin/vectorcraft-nightly", "usr/lib/vectorcraft-nightly/vectorcraft-bin"),
    "appimage": ("AppRun", "usr/bin/vectorcraft-bin"),
    "arch": ("bin/vectorcraft", "bin/.vectorcraft-nightly/vectorcraft-bin"),
    "flatpak": ("bin/vectorcraft", "libexec/vectorcraft-bin"),
}


class Launchers(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name).resolve()
        self.home = self.root / "home with space"
        self.home.mkdir()
        self.env = {"HOME": str(self.home), "PATH": "/usr/bin:/bin"}
        self.profile = f"{self.home}/.local/share/vectorcraft-nightly"

    def place(self, name):
        launcher_rel, gui_rel = PLACEMENTS[name]
        launcher = self.root / name / launcher_rel
        gui = self.root / name / gui_rel
        gui.parent.mkdir(parents=True, exist_ok=True)
        gui.write_text(RECORDER, encoding="utf-8")
        gui.chmod(0o755)
        launcher.parent.mkdir(parents=True, exist_ok=True)
        launcher.write_bytes(LAUNCHER.read_bytes())
        launcher.chmod(0o755)
        return launcher, gui

    def run_launcher(self, launcher, *args, env=None, extra_path=None, unset=()):
        environment = dict(self.env, **(env or {}))
        for name in unset:
            environment.pop(name, None)
        if extra_path:
            environment["PATH"] = f"{extra_path}:{environment['PATH']}"
        # Run from somewhere else entirely: the launcher resolves the GUI from its own location.
        result = subprocess.run(
            [str(launcher), *args],
            cwd=str(self.root),
            env=environment,
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertEqual(result.returncode, 0, f"{launcher.name} {args}: {result.stderr}")
        return result.stdout.splitlines()

    def expect(self, launcher, expected, *args, env=None, extra_path=None, unset=()):
        lines = self.run_launcher(launcher, *args, env=env, extra_path=extra_path, unset=unset)
        self.assertEqual(lines, expected, f"{launcher} {list(args)}")

    def test_every_placement_starts_the_gui_with_the_channel_profile(self):
        for name in PLACEMENTS:
            with self.subTest(placement=name):
                launcher, _ = self.place(name)
                self.expect(
                    launcher,
                    ["config=<>", "<--launch-profile>", "<nightly-v1>", "<--profile-root>", f"<{self.profile}>", "<-->"],
                )

    def test_files_keep_their_names_and_spaces_after_the_delimiter(self):
        for name in PLACEMENTS:
            with self.subTest(placement=name):
                launcher, _ = self.place(name)
                self.expect(
                    launcher,
                    ["config=<>", "<--launch-profile>", "<nightly-v1>", "<--profile-root>", f"<{self.profile}>", "<-->", "<drawing with spaces.svg>", "<--other-name.svg>"],
                    "drawing with spaces.svg",
                    "--other-name.svg",
                )

    def test_the_profile_follows_xdg_data_home(self):
        for name in PLACEMENTS:
            with self.subTest(placement=name):
                launcher, _ = self.place(name)
                xdg = f"{self.root}/xdg with space"
                self.expect(
                    launcher,
                    ["config=<>", "<--launch-profile>", "<nightly-v1>", "<--profile-root>", f"<{xdg}/vectorcraft-nightly>", "<-->"],
                    env={"XDG_DATA_HOME": xdg},
                )

    def test_version_and_help_queries_reach_the_gui_unwrapped(self):
        for name in PLACEMENTS:
            for query in ("--version", "-V", "--help", "-h"):
                with self.subTest(placement=name, query=query):
                    launcher, _ = self.place(name)
                    self.expect(launcher, ["config=<>", f"<{query}>"], query)

    def test_a_version_or_help_query_answers_even_without_a_home(self):
        # Nothing to build a default profile root from, but a query needs none.
        for name in PLACEMENTS:
            for query in ("--version", "--help"):
                with self.subTest(placement=name, query=query):
                    launcher, _ = self.place(name)
                    self.expect(launcher, ["config=<>", f"<{query}>"], query, unset=("HOME", "XDG_DATA_HOME"))

    def test_the_launchers_own_protocol_and_delimiter_pass_through(self):
        for name in PLACEMENTS:
            with self.subTest(placement=name):
                launcher, _ = self.place(name)
                root_arg = f"{self.root}/launcher root"
                self.expect(
                    launcher,
                    ["config=<>", "<--launch-profile>", "<nightly-v1>", "<--profile-root>", f"<{root_arg}>", "<--version>"],
                    "--launch-profile",
                    "nightly-v1",
                    "--profile-root",
                    root_arg,
                    "--version",
                )
                # A caller that already wrote the delimiter must not get a second one.
                self.expect(
                    launcher,
                    ["config=<>", "<--launch-profile>", "<nightly-v1>", "<--profile-root>", f"<{self.profile}>", "<-->", "<file.svg>"],
                    "--",
                    "file.svg",
                )

    def test_a_path_invocation_finds_the_gui_beside_it(self):
        for name in PLACEMENTS:
            with self.subTest(placement=name):
                launcher, _ = self.place(name)
                environment = dict(self.env, PATH=f"{launcher.parent}:/usr/bin:/bin")
                result = subprocess.run([launcher.name], cwd=str(self.root), env=environment, capture_output=True, text=True, timeout=30)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    result.stdout.splitlines(),
                    ["config=<>", "<--launch-profile>", "<nightly-v1>", "<--profile-root>", f"<{self.profile}>", "<-->"],
                )

    def test_a_missing_gui_is_refused_rather_than_started_elsewhere(self):
        for name in PLACEMENTS:
            with self.subTest(placement=name):
                launcher, gui = self.place(name)
                gui.unlink()
                result = subprocess.run([str(launcher)], cwd=str(self.root), env=self.env, capture_output=True, text=True, timeout=30)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("app binary is missing", result.stderr)


class TarLauncher(unittest.TestCase):
    """The plain tar's entrypoint: the shared template, rendered for VectorCraft."""

    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name).resolve()
        self.home = self.root / "home with space"
        self.home.mkdir()
        self.env = {"HOME": str(self.home), "PATH": "/usr/bin:/bin"}

    def place(self):
        """The kit's flat tar: the `vectorcraft` launcher beside the `vectorcraft-bin` GUI."""
        tree = self.root / "tar"
        tree.mkdir()
        (tree / "vectorcraft-bin").write_text(RECORDER, encoding="utf-8")
        (tree / "vectorcraft-bin").chmod(0o755)
        (tree / "vectorcraft").write_bytes(TAR_LAUNCHER.read_bytes())
        (tree / "vectorcraft").chmod(0o755)
        return tree / "vectorcraft"

    def lines(self, launcher, *args, env=None):
        result = subprocess.run([str(launcher), *args], cwd=str(self.root), env=env or self.env,
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.splitlines()

    def test_every_argument_follows_the_profile_tokens(self):
        launcher = self.place()
        root = f"{self.home}/.local/share/{TAR_PROFILE}"
        tokens = ["config=<>", "<--launch-profile>", "<nightly-v1>", "<--profile-root>", f"<{root}>"]
        self.assertEqual(self.lines(launcher), [*tokens, "<-->"])
        self.assertEqual(self.lines(launcher, "drawing with spaces.svg", "--other-name.svg"),
                         [*tokens, "<-->", "<drawing with spaces.svg>", "<--other-name.svg>"])

    def test_the_profile_follows_xdg_data_home(self):
        launcher = self.place()
        xdg = self.root / "xdg with space"
        env = dict(self.env, XDG_DATA_HOME=str(xdg))
        self.assertEqual(self.lines(launcher, "drawing.svg", env=env),
                         ["config=<>", "<--launch-profile>", "<nightly-v1>", "<--profile-root>",
                          f"<{xdg / TAR_PROFILE}>", "<-->", "<drawing.svg>"])
        self.assertFalse(xdg.exists(), "the launcher creates no folder of its own")

    def test_a_version_query_asks_the_gui_in_the_launcher_form(self):
        launcher = self.place()
        root = f"{self.home}/.local/share/{TAR_PROFILE}"
        for query in ("--version", "-V"):
            with self.subTest(query=query):
                self.assertEqual(self.lines(launcher, query),
                                 ["config=<>", "<--launch-profile>", "<nightly-v1>", "<--profile-root>",
                                  f"<{root}>", "<--version>"])

    def test_a_missing_gui_fails_rather_than_starting_another_binary(self):
        launcher = self.place()
        (self.root / "tar/vectorcraft-bin").unlink()
        result = subprocess.run([str(launcher)], cwd=str(self.root), env=self.env,
                                capture_output=True, text=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")

    def test_the_bytes_are_the_shared_template_rendered_for_vectorcraft(self):
        data = TAR_LAUNCHER.read_bytes()
        self.assertEqual(hashlib.sha256(data).hexdigest(), TAR_LAUNCHER_SHA256,
                         "the tar launcher must stay byte for byte scripts/nightly-launcher.sh.in rendered for vectorcraft")
        self.assertIn(b'target="$dir/vectorcraft-bin"', data)
        self.assertIn(f'{TAR_PROFILE}"'.encode(), data)

    def test_the_flat_tar_stages_the_template(self):
        source = (HERE / "nightly.sh").read_text(encoding="utf-8")
        self.assertIn('"$ROOT/packaging/linux/nightly-tar-launch.sh" "$STAGE/vectorcraft"', source)
        self.assertNotIn('"$ROOT/packaging/linux/nightly-launch.sh" "$STAGE/vectorcraft"', source)


if __name__ == "__main__":
    unittest.main()
