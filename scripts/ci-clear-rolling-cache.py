#!/usr/bin/env python3
"""The nightly cold run deletes the rolling crates entries just before it saves its own.

    python3 scripts/ci-clear-rolling-cache.py        needs GH_TOKEN, GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT

The cold run (ci.yml, `cold-build`) never restores the rolling entry, so the old one only adds to
the cache while the new one is saved: two entries of about 4.6 GB each, beside the others, go over
the repository's 10 GB limit and GitHub evicts the corpus, SonarCloud and Windows ARM64 entries. The
gate's `cache warm` leg saves in its `Post Setup` step; this deletes the old entries when that step
starts, then waits for the step to finish. Pull requests and pushes that start in the minutes
between restore nothing from the deleted entry and build cold once. Deleting is the only write; the
entry the cold run saves does not exist yet when this runs. Only entries created before this run
started are deleted: a push or pull request run in the same minutes may have saved an entry of its
own, and the cold run's own save is never touched. A leg that failed or was canceled before its
save step started saves nothing: its old entries stay, and this exits 1, so "Re-run failed jobs"
runs it again beside the re-run leg. A save step that is canceled or fails after the old entries
were deleted leaves no rolling entry, so the next run starts cold, and this exits 1 too. A failure
in a later post step does not count once the save step has succeeded. This also exits 1 when the
leg has not started START_SECONDS after this script started, and when the leg has not reached its
save, or its save has not finished, GIVE_UP_SECONDS after the leg started. An absent warm leg is treated as finished only
when the most recent earlier attempt that lists it reports success and the current attempt lists
no `cache prune` job, which runs from the start of "Re-run all jobs". Otherwise this keeps polling
because reusable-workflow dependencies may still be scheduling it.
"""

from datetime import datetime, timezone
import json
import os
import re
import subprocess
import sys
import time

REF = 'refs/heads/main'
WARM_JOB = re.compile(r'^gate / cache warm \(')
# The jobs the gate's legs need (craft-common-actions .github/workflows/gate.yml): when one of
# them fails or is canceled, the legs are skipped and no warm leg is listed.
NEEDED_JOBS = re.compile(r'^gate / (select legs$|build \()')
PRUNE_JOB = 'cache prune'
ROLLING = '-gate-crates-'
# The gate-setup action's post step, which saves the dependency cache (craft-common-actions
# actions/gate-setup, under the gate's `Setup` step).
SAVE_STEP = 'Post Setup'
POLL_SECONDS = 20
PAGE_SIZE = 100
# Longer than the gate's `timeout-minutes` in ci.yml and timed from the leg's own start, so a cold
# leg that starts late and runs to its limit still reaches its save before this gives up.
GIVE_UP_SECONDS = 160 * 60
# How long the leg may take to start, timed from this script's start: the leg waits for
# `gate / select legs` and then for a runner, which took 44 minutes on a push to main on
# 2026-11-28. The `cold-cache-clear` job's `timeout-minutes` in ci.yml is longer than
# START_SECONDS and GIVE_UP_SECONDS together, so this exits with its own error before the job
# times out.
START_SECONDS = 120 * 60
STOPPED = ('failure', 'cancelled', 'timed_out')
# Steps with continue-on-error in craft-common-actions .github/workflows/gate.yml.
TOLERATED_FAILURES = ('Trim the cache entry to this build',)
NO_SAVE = 'the cache warm leg failed or was canceled and saves no entry; nothing deleted. Re-running the failed jobs runs this job again with the leg'
SAVE_LOST = 'the cache warm leg did not finish its save after the old rolling entries were deleted; the next run starts cold'


def save_step(job):
    return next((s for s in job.get('steps', []) if s['name'] == SAVE_STEP), None)


def saving(job):
    """True once the job's save step has started."""
    step = save_step(job)
    return step is not None and step['status'] in ('in_progress', 'completed')


def failed(job):
    """True when a step before the post steps failed or was canceled: the leg then saves no entry."""
    return any(not s['name'].startswith('Post ') and s['name'] not in TOLERATED_FAILURES and s.get('conclusion') in STOPPED for s in job.get('steps', []))


def saves_nothing(job):
    """True when the leg saves no entry: a step before the save stopped, the save step did not
    succeed, or the job ended before its save step finished."""
    if failed(job):
        return True
    step = save_step(job)
    if step is not None and step['status'] == 'completed':
        return step.get('conclusion') != 'success'
    return job.get('status') == 'completed'


def save_finished(job):
    """True once the save step has completed or the job has ended."""
    step = save_step(job)
    return job.get('status') == 'completed' or (step is not None and step['status'] == 'completed')


def started_at(job):
    """The warm leg's start time, or None while the leg has not started."""
    started = job.get('started_at') if job and job.get('status') in ('in_progress', 'completed') else None
    return parse_time(started) if started else None


def start_overdue(job, now, began):
    """True once START_SECONDS have passed since `began` (this script's start) and the warm leg
    has not started."""
    return started_at(job) is None and (now - began).total_seconds() > START_SECONDS


def started_late(job, began):
    """True when the warm leg started more than START_SECONDS after `began` (this script's start)."""
    started = started_at(job)
    return started is not None and (started - began).total_seconds() > START_SECONDS


def gave_up(job, now):
    """True once GIVE_UP_SECONDS have passed since the warm leg started."""
    started = started_at(job)
    return started is not None and (now - started).total_seconds() > GIVE_UP_SECONDS


def all_caches(fetch):
    """Every cache entry, one page at a time: `fetch(page)` returns one page of actions/caches."""
    entries, page = [], 1
    while True:
        batch = (fetch(page) or {}).get('actions_caches', [])
        entries += batch
        if len(batch) < PAGE_SIZE:
            return entries
        page += 1


def warm_job(jobs):
    return next((j for j in jobs if WARM_JOB.match(j['name'])), None)


def legs_stopped(jobs):
    """True when a job the gate's legs need failed or was canceled, so the warm leg never starts."""
    return any(NEEDED_JOBS.match(j['name']) and j['status'] == 'completed' and j.get('conclusion') in STOPPED for j in jobs)


def not_in_attempt(jobs, earlier_jobs):
    """True when no prune job is listed and the most recent earlier warm leg succeeded."""
    if any(j['name'] == PRUNE_JOB for j in jobs):
        return False
    for attempt in sorted(earlier_jobs, reverse=True):
        job = warm_job(earlier_jobs[attempt])
        if job is not None:
            return job.get('conclusion') == 'success'
    return False


def rolling_entries(caches):
    return [c for c in caches if ROLLING in c['key']]


def created_before(entries, started):
    """The entries created before `started`, an ISO 8601 timestamp (`run_started_at`)."""
    limit = parse_time(started)
    return [c for c in entries if parse_time(c['created_at']) < limit]


def parse_time(text):
    return datetime.fromisoformat(text.replace('Z', '+00:00'))


def gh(*args):
    out = subprocess.run(['gh', 'api', *args], check=True, capture_output=True, text=True).stdout
    return json.loads(out) if out.strip() else None


def main():
    repo, run, attempt = os.environ['GITHUB_REPOSITORY'], os.environ['GITHUB_RUN_ID'], os.environ.get('GITHUB_RUN_ATTEMPT', '1')
    began = datetime.now(timezone.utc)
    jobs_url = f'repos/{repo}/actions/runs/{run}/attempts/{attempt}/jobs?per_page=100'
    earlier_jobs = {}
    while True:
        jobs = gh(jobs_url) or {}
        job = warm_job(jobs.get('jobs', []))
        if started_late(job, began):
            print(f'::error::the cache warm leg did not start within {START_SECONDS // 60} minutes; nothing deleted')
            return 1
        if job and saving(job):
            break
        if job is None:
            for earlier in range(int(attempt) - 1, 0, -1):
                if earlier not in earlier_jobs:
                    previous = gh(f'repos/{repo}/actions/runs/{run}/attempts/{earlier}/jobs?per_page=100') or {}
                    earlier_jobs[earlier] = previous.get('jobs', [])
                if warm_job(earlier_jobs[earlier]) is not None:
                    break
            if not_in_attempt(jobs.get('jobs', []), earlier_jobs):
                print('the cache warm leg is not part of this attempt (it succeeded in an earlier one); nothing deleted')
                return
            if legs_stopped(jobs.get('jobs', [])):
                print('::error::the gate stopped before its cache warm leg started; nothing deleted')
                return 1
        if job and job['status'] == 'completed':
            if job.get('conclusion') in STOPPED:
                print(f'::error::{NO_SAVE}')
                return 1
            print('the cache warm leg finished without a save step; nothing deleted')
            return
        now = datetime.now(timezone.utc)
        if start_overdue(job, now, began):
            print(f'::error::the cache warm leg did not start within {START_SECONDS // 60} minutes; nothing deleted')
            return 1
        if gave_up(job, now):
            print('::error::the cache warm leg never reached its save; nothing deleted')
            return 1
        time.sleep(POLL_SECONDS)
    if saves_nothing(job):
        print(f'::error::{NO_SAVE}')
        return 1
    started = gh(f'repos/{repo}/actions/runs/{run}')['run_started_at']
    caches = all_caches(lambda page: gh(f'repos/{repo}/actions/caches?ref={REF}&sort=created_at&direction=asc&per_page={PAGE_SIZE}&page={page}'))
    for c in created_before(rolling_entries(caches), started):
        print(f"delete {c['key']} ({c['size_in_bytes'] / 1e9:.2f} GB)")
        result = subprocess.run(['gh', 'api', '-X', 'DELETE', f"repos/{repo}/actions/caches/{c['id']}"], capture_output=True, text=True)
        if result.returncode:
            if '(HTTP 404)' in result.stderr:
                print(f"already deleted {c['key']}")
            else:
                print(result.stderr, end='', file=sys.stderr)
                return result.returncode
    while not save_finished(job):
        if gave_up(job, datetime.now(timezone.utc)):
            print(f'::error::{SAVE_LOST}')
            return 1
        time.sleep(POLL_SECONDS)
        job = warm_job((gh(jobs_url) or {}).get('jobs', [])) or job
    if saves_nothing(job):
        print(f'::error::{SAVE_LOST}')
        return 1
    print('the cache warm leg saved its entry')


if __name__ == '__main__':
    sys.exit(main())
