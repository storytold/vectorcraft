#!/usr/bin/env python3
"""The nightly cold run deletes the rolling crates entries just before it saves its own.

    python3 scripts/ci-clear-rolling-cache.py        needs GH_TOKEN, GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT

The cold run (ci.yml, `cold-build`) never restores the rolling entry, so the old one only adds to
the cache while the new one is saved: two entries of about 4.6 GB each, beside the others, go over
the repository's 10 GB limit and GitHub evicts the corpus, SonarCloud and Windows ARM64 entries. The
gate's `cache warm` leg saves in its `Post` steps; this deletes the old entries when the first of
them starts. Pull requests and pushes that start in the minutes between restore nothing from the
deleted entry and build cold once. Deleting is the only write; the entry the cold run saves does not
exist yet when this runs. Only entries created before this run started are deleted: a push or pull
request run in the same minutes may have saved an entry of its own, and the cold run's own save is
never touched. A leg that failed or was canceled saves nothing: its old entries stay, and this
exits 1, so "Re-run failed jobs" runs it again beside the re-run leg. It also exits 1 when the leg
has not reached its save after GIVE_UP_SECONDS. An absent warm leg is treated as finished only
when the most recent earlier attempt that lists it reports success and the current attempt lists
no `cache prune` job, which runs from the start of "Re-run all jobs". Otherwise this keeps polling
because reusable-workflow dependencies may still be scheduling it.
"""

from datetime import datetime
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
POLL_SECONDS = 20
# Longer than the gate's `timeout-minutes` in ci.yml, so a cold leg that runs to its limit still
# reaches its save before this gives up.
GIVE_UP_SECONDS = 160 * 60
STOPPED = ('failure', 'cancelled', 'timed_out')
# Steps with continue-on-error in craft-common-actions .github/workflows/gate.yml.
TOLERATED_FAILURES = ('Trim the cache entry to this build',)
NO_SAVE = 'the cache warm leg failed or was canceled and saves no entry; nothing deleted. Re-running the failed jobs runs this job again with the leg'


def saving(job):
    """True once the job's post steps (the cache save) have started."""
    return any(s['name'].startswith('Post ') and s['status'] in ('in_progress', 'completed') for s in job.get('steps', []))


def failed(job):
    """True when a step before the post steps failed or was canceled: the leg then saves no entry."""
    return any(not s['name'].startswith('Post ') and s['name'] not in TOLERATED_FAILURES and s.get('conclusion') in STOPPED for s in job.get('steps', []))


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
    deadline = time.monotonic() + GIVE_UP_SECONDS
    earlier_jobs = {}
    while True:
        jobs = gh(f'repos/{repo}/actions/runs/{run}/attempts/{attempt}/jobs?per_page=100') or {}
        job = warm_job(jobs.get('jobs', []))
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
        if time.monotonic() > deadline:
            print('::error::the cache warm leg never reached its save; nothing deleted')
            return 1
        time.sleep(POLL_SECONDS)
    if failed(job):
        print(f'::error::{NO_SAVE}')
        return 1
    started = gh(f'repos/{repo}/actions/runs/{run}')['run_started_at']
    caches = (gh(f'repos/{repo}/actions/caches?ref={REF}&per_page=100') or {}).get('actions_caches', [])
    for c in created_before(rolling_entries(caches), started):
        print(f"delete {c['key']} ({c['size_in_bytes'] / 1e9:.2f} GB)")
        result = subprocess.run(['gh', 'api', '-X', 'DELETE', f"repos/{repo}/actions/caches/{c['id']}"], capture_output=True, text=True)
        if result.returncode:
            if '(HTTP 404)' in result.stderr:
                print(f"already deleted {c['key']}")
            else:
                print(result.stderr, end='', file=sys.stderr)
                return result.returncode


if __name__ == '__main__':
    sys.exit(main())
