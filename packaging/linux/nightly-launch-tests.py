#!/usr/bin/env python3
"""Synthetic launcher tests: every VectorCraft Nightly placement starts the GUI the flavor's way.

No build and no window: a recorder script stands in for the GUI, so the argv and environment the
launcher passes are read directly.
"""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
LAUNCHER = HERE / "nightly-launch.sh"

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


if __name__ == "__main__":
    unittest.main()
