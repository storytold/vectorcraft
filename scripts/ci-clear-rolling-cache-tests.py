#!/usr/bin/env python3
"""Checks for ci-clear-rolling-cache.py with recorded GitHub API answers; no network.

    python3 scripts/ci-clear-rolling-cache-tests.py
"""
import contextlib
from datetime import datetime, timedelta, timezone
import importlib.util
import io
import os
from pathlib import Path
import re
import subprocess
import unittest
from unittest import mock

_spec = importlib.util.spec_from_file_location("clear", Path(__file__).with_name("ci-clear-rolling-cache.py"))
clear = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(clear)

WARM = "gate / cache warm (ubuntu-24.04)"
ME = [{"name": "cold cache clear", "status": "in_progress"}]


def warm_job(status, conclusion=None, steps=(), started_at="2026-11-28T03:40:00Z"):
    return {
        "name": WARM,
        "status": status,
        "conclusion": conclusion,
        "started_at": started_at,
        "steps": [{"name": n, "status": s, "conclusion": c} for n, s, c in steps],
    }


def earlier(conclusion):
    return [{"name": WARM, "conclusion": conclusion}, {"name": "cold cache clear", "conclusion": "failure"}]


class SaveStep(unittest.TestCase):
    def test_the_leg_is_saving_once_its_save_step_starts(self):
        self.assertFalse(clear.saving(warm_job("in_progress", steps=[("Setup", "completed", "success"), ("Gate", "in_progress", None)])))
        self.assertTrue(clear.saving(warm_job("in_progress", steps=[("Gate", "completed", "success"), ("Post Setup", "in_progress", None)])))

    def test_other_post_steps_are_not_the_save(self):
        job = warm_job("in_progress", steps=[("Gate", "completed", "success"), ("Post Harden runner", "in_progress", None)])
        self.assertFalse(clear.saving(job))

    def test_a_leg_canceled_during_its_save_saves_nothing(self):
        job = warm_job("completed", "cancelled", [("Gate", "completed", "success"), ("Post Setup", "completed", "cancelled")])
        self.assertTrue(clear.saving(job))
        self.assertTrue(clear.saves_nothing(job))

    def test_a_leg_that_ended_with_its_save_still_running_saves_nothing(self):
        job = warm_job("completed", "cancelled", [("Gate", "completed", "success"), ("Post Setup", "in_progress", None)])
        self.assertTrue(clear.saves_nothing(job))

    def test_a_running_or_finished_save_counts_as_saved(self):
        running = warm_job("in_progress", steps=[("Gate", "completed", "success"), ("Post Setup", "in_progress", None)])
        done = warm_job("completed", "failure", [("Gate", "completed", "success"), ("Post Setup", "completed", "success"),
                                                  ("Post Harden runner", "completed", "failure")])
        self.assertFalse(clear.saves_nothing(running))
        self.assertFalse(clear.saves_nothing(done))

    def test_a_failed_step_before_the_save_saves_nothing(self):
        job = lambda c: warm_job("in_progress", steps=[("Gate", "completed", c), ("Post Setup", "in_progress", None)])
        self.assertTrue(clear.saves_nothing(job("failure")))
        self.assertTrue(clear.saves_nothing(job("cancelled")))
        self.assertFalse(clear.saves_nothing(job("success")))
        tolerated = warm_job("in_progress", steps=[("Gate", "completed", "success"), ("Trim the cache entry to this build", "completed", "failure"),
                                                   ("Post Setup", "in_progress", None)])
        self.assertFalse(clear.saves_nothing(tolerated))


class Deadline(unittest.TestCase):
    began = datetime(2026, 11, 28, 3, 37, tzinfo=timezone.utc)

    def test_the_wait_is_timed_from_the_leg_start(self):
        late = warm_job("in_progress", started_at="2026-11-28T04:00:00Z")
        before_limit = clear.parse_time("2026-11-28T04:00:00Z") + timedelta(seconds=clear.GIVE_UP_SECONDS - 60)
        after_limit = clear.parse_time("2026-11-28T04:00:00Z") + timedelta(seconds=clear.GIVE_UP_SECONDS + 60)
        self.assertGreater((before_limit - self.began).total_seconds(), clear.GIVE_UP_SECONDS)
        self.assertFalse(clear.gave_up(late, before_limit))
        self.assertTrue(clear.gave_up(late, after_limit))
        self.assertFalse(clear.start_overdue(late, after_limit, self.began))

    def test_a_leg_that_never_starts_is_timed_from_the_script_start(self):
        queued = warm_job("queued", started_at="2026-11-28T03:38:00Z")
        before_limit = self.began + timedelta(seconds=clear.START_SECONDS - 60)
        after_limit = self.began + timedelta(seconds=clear.START_SECONDS + 60)
        self.assertFalse(clear.start_overdue(None, before_limit, self.began))
        self.assertTrue(clear.start_overdue(None, after_limit, self.began))
        self.assertTrue(clear.start_overdue(queued, after_limit, self.began))
        self.assertFalse(clear.gave_up(queued, after_limit))

    def test_a_leg_that_starts_after_the_start_limit_fails(self):
        at = lambda seconds: warm_job("in_progress", started_at=(self.began + timedelta(seconds=seconds)).isoformat())
        self.assertTrue(clear.started_late(at(clear.START_SECONDS + 5), self.began))
        self.assertFalse(clear.started_late(at(clear.START_SECONDS - 10), self.began))
        self.assertFalse(clear.started_late(warm_job("queued"), self.began))
        self.assertFalse(clear.started_late(None, self.began))


class Workflow(unittest.TestCase):
    ci = (Path(__file__).resolve().parent.parent / ".github" / "workflows" / "ci.yml").read_text()

    def timeout_minutes(self, job):
        """The first `timeout-minutes` under `job` in ci.yml."""
        lines = self.ci.splitlines()
        start = lines.index(f"  {job}:") + 1
        for line in lines[start:]:
            if re.match(r"  \S", line):
                break
            match = re.match(r"\s+timeout-minutes: (\d+)$", line)
            if match:
                return int(match.group(1))
        self.fail(f"no timeout-minutes under {job}")

    def test_the_job_outlasts_the_start_wait_and_the_save_wait(self):
        # Five minutes are left for the steps before the script and for the deletion.
        spare = self.timeout_minutes("cold-cache-clear") * 60 - clear.START_SECONDS - clear.GIVE_UP_SECONDS
        self.assertGreaterEqual(spare, 5 * 60)

    def test_the_save_wait_outlasts_the_gate(self):
        self.assertGreater(clear.GIVE_UP_SECONDS, self.timeout_minutes("gate") * 60)


class Jobs(unittest.TestCase):
    def test_only_the_warm_leg_counts(self):
        jobs = [{"name": "gate / test 1/5 (ubuntu-24.04)"}, {"name": "gate / build (${{ matrix.os }})"}, {"name": WARM}]
        self.assertEqual(clear.warm_job(jobs), jobs[2])

    def test_a_rerun_of_this_job_alone_skips_a_leg_that_succeeded(self):
        self.assertTrue(clear.not_in_attempt(ME, {1: earlier("success")}))
        self.assertTrue(clear.not_in_attempt(ME, {1: earlier("success"), 2: [{"name": "cold cache clear"}]}))
        self.assertTrue(clear.not_in_attempt(ME, {1: earlier("failure"), 2: [{"name": "cold cache clear"}], 3: earlier("success")}))

    def test_a_leg_that_did_not_succeed_is_waited_for(self):
        self.assertFalse(clear.not_in_attempt(ME, {1: earlier("cancelled")}))
        self.assertFalse(clear.not_in_attempt(ME, {}))
        self.assertFalse(clear.not_in_attempt(ME, {1: earlier("success"), 2: earlier("failure")}))

    def test_a_rerun_of_all_jobs_is_waited_for(self):
        self.assertFalse(clear.not_in_attempt(ME + [{"name": "cache prune", "status": "in_progress"}], {1: earlier("success")}))

    def test_a_failed_select_or_build_stops_the_wait(self):
        done = lambda n, c: {"name": n, "status": "completed", "conclusion": c}
        self.assertTrue(clear.legs_stopped([done("gate / select legs", "failure")]))
        self.assertTrue(clear.legs_stopped([done("gate / select legs", "success"), done("gate / build (ubuntu-24.04)", "cancelled")]))
        self.assertFalse(clear.legs_stopped([done("gate / select legs", "success"), done("gate / build (${{ matrix.os }})", "skipped")]))
        self.assertFalse(clear.legs_stopped([done("gate / test 1/5 (ubuntu-24.04)", "failure")]))


class Caches(unittest.TestCase):
    def test_every_page_is_read(self):
        entries = [{"id": n, "key": f"v2-rust-a013-gate-crates-Linux-x64-{n}"} for n in range(2 * clear.PAGE_SIZE + 3)]
        pages = []

        def fetch(page):
            pages.append(page)
            start = (page - 1) * clear.PAGE_SIZE
            return {"actions_caches": entries[start:start + clear.PAGE_SIZE]}

        self.assertEqual(clear.all_caches(fetch), entries)
        self.assertEqual(pages, [1, 2, 3])

    def test_only_rolling_crates_entries_are_deleted(self):
        caches = [
            {"key": "v2-rust-a013-gate-crates-Linux-x64-46055934-01d60c35"},
            {"key": "v2-rust-a013-gate-Linux-x64-46055934-01d60c35"},
            {"key": "v1-affinity-rust-a013-gate-Linux-x64-b541f517-01d60c35"},
            {"key": "v0-rust-build-Linux-x64-1fd829d9-01d60c35"},
        ]
        self.assertEqual(clear.rolling_entries(caches), caches[:1])

    def test_entries_created_after_the_run_started_are_kept(self):
        rolling = lambda created: {"key": "v2-rust-ab-gate-crates-Linux-x64-1", "created_at": created}
        old, same_minute_push = rolling("2026-11-27T03:00:00.5Z"), rolling("2026-11-28T03:40:00Z")
        self.assertEqual(clear.created_before([old, same_minute_push], "2026-11-28T03:37:10Z"), [old])


class Clock(datetime):
    @classmethod
    def now(cls, tz=None):
        return datetime(2026, 11, 28, 4, 0, tzinfo=timezone.utc)


class AfterDeletion(unittest.TestCase):
    SAVING = [("Gate", "completed", "success"), ("Post Setup", "in_progress", None)]

    def run_main(self, after):
        """Runs main() with the warm leg's save starting at the first poll and `after` at every later poll."""
        polls = [warm_job("in_progress", steps=self.SAVING), after]
        deleted = []

        def gh(url):
            if "/jobs?" in url:
                return {"jobs": ME + [polls.pop(0) if len(polls) > 1 else polls[0]]}
            if "/caches?" in url:
                old = {"id": 7, "key": "v2-rust-ab-gate-crates-Linux-x64-1", "created_at": "2026-11-27T03:40:00Z", "size_in_bytes": 4.6e9}
                return {"actions_caches": [old]}
            return {"run_started_at": "2026-11-28T03:37:10Z"}

        def run(args, **kwargs):
            deleted.append(args[-1])
            return subprocess.CompletedProcess(args, 0, "", "")

        env = {"GITHUB_REPOSITORY": "storytold/vectorcraft", "GITHUB_RUN_ID": "1", "GITHUB_RUN_ATTEMPT": "1"}
        with mock.patch.object(clear, "gh", gh), mock.patch.object(clear.subprocess, "run", run), mock.patch.object(clear.time, "sleep"), \
                mock.patch.object(clear, "datetime", Clock), mock.patch.dict(os.environ, env), contextlib.redirect_stdout(io.StringIO()):
            status = clear.main()
        self.assertEqual(deleted, ["repos/storytold/vectorcraft/actions/caches/7"])
        return status

    def test_a_save_that_succeeds_after_the_deletion_exits_0(self):
        saved = warm_job("completed", "success", [("Gate", "completed", "success"), ("Post Setup", "completed", "success")])
        later_post_failure = warm_job("completed", "failure", [("Gate", "completed", "success"), ("Post Setup", "completed", "success"),
                                                                ("Post Harden runner", "completed", "failure")])
        self.assertFalse(self.run_main(saved))
        self.assertFalse(self.run_main(later_post_failure))

    def test_a_save_canceled_after_the_deletion_exits_1(self):
        canceled = warm_job("completed", "cancelled", [("Gate", "completed", "success"), ("Post Setup", "completed", "cancelled")])
        self.assertEqual(self.run_main(canceled), 1)

    def test_a_leg_that_starts_late_and_saves_deletes_nothing(self):
        # Clock.now() is 04:00, so the leg starts 120 minutes 5 seconds after the script.
        late = warm_job("in_progress", steps=self.SAVING, started_at="2026-11-28T06:00:05Z")
        deleted = []

        def run(args, **kwargs):
            deleted.append(args[-1])
            return subprocess.CompletedProcess(args, 0, "", "")

        env = {"GITHUB_REPOSITORY": "storytold/vectorcraft", "GITHUB_RUN_ID": "1", "GITHUB_RUN_ATTEMPT": "1"}
        with mock.patch.object(clear, "gh", lambda url: {"jobs": ME + [late]}), mock.patch.object(clear.subprocess, "run", run), \
                mock.patch.object(clear.time, "sleep"), mock.patch.object(clear, "datetime", Clock), mock.patch.dict(os.environ, env), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(clear.main(), 1)
        self.assertEqual(deleted, [])

    def test_a_save_that_fails_after_the_deletion_exits_1(self):
        failed = warm_job("completed", "failure", [("Gate", "completed", "success"), ("Post Setup", "completed", "failure")])
        self.assertEqual(self.run_main(failed), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
