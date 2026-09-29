"""Check an aggregate gate job: its needs, their results and removed jobs.

usage: python .github/ci/check_gates.py WORKFLOW GATE [--exempt JOB ...] [--no-results]

Run from the repository root by the gate job of WORKFLOW, with RESULTS set to
toJSON(needs) and, on pull requests, PR_BODY set to the pull request body and
the merge commit checked out with its parents (fetch-depth: 2). It fails when:

- a needed job did not succeed (skipped with --no-results, where the gate
  applies its own result rules);
- a job in WORKFLOW other than GATE and the exempt jobs is missing from needs;
- on a pull request, a job on the base branch's copy of WORKFLOW is gone, and
  the pull request body has no line "removed-jobs: WORKFLOW#JOB" for it;
- a line at job-key indentation in the jobs block is not a job key it can read.
"""

import json
import os
import string
import subprocess
import sys

ID_START = set(string.ascii_letters + "_")
ID_CHARS = ID_START | set(string.digits + "-")


def job_key(text: str) -> str | None:
    """Return the job id a line (without its indentation) declares, or None if it is not a plain key."""
    quote = text[0] if text[:1] in ("'", '"') else ""
    rest = text[len(quote) :]
    length = 0
    while length < len(rest) and rest[length] in ID_CHARS:
        length += 1
    name, rest = rest[:length], rest[length:]
    if not name or name[0] not in ID_START or not rest.startswith(quote):
        return None
    rest = rest[len(quote) :].lstrip(" \t")
    if not rest.startswith(":") or rest[1:2] not in ("", " ", "\t"):
        return None
    return name


def job_ids(text: str) -> set[str]:
    """Return the job ids of a workflow, failing on any job-level line it cannot read."""
    parts = text.split("\njobs:\n", 1)
    if len(parts) != 2:
        sys.exit("Cannot find the jobs block; the gate check needs updating")
    ids = set()
    for line in parts[1].splitlines():
        if line[:1] not in ("", " ", "#"):
            break  # the next top-level key ends the jobs block
        if line.startswith("  ") and line[2:3] not in ("", " ", "#"):
            name = job_key(line[2:])
            if name is None:
                sys.exit(f"Cannot read the job key {line!r}; the gate check needs updating")
            ids.add(name)
    return ids


def workflow_file(argument: str) -> str:
    """Return the path of WORKFLOW, which must name a file directly in .github/workflows."""
    # The path comes from the directory listing, never from the argument itself.
    prefix = ".github/workflows/"
    listing = {entry.name: entry.path for entry in os.scandir(prefix) if entry.is_file()}
    name = argument[len(prefix) :] if argument.startswith(prefix) else ""
    if name not in listing:
        sys.exit(f"{argument} is not a file in .github/workflows")
    return listing[name]


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def main(argv: list[str]) -> None:
    args = [a for a in argv if a != "--no-results"]
    exempt: set[str] = set()
    if "--exempt" in args:
        index = args.index("--exempt")
        args, exempt = args[:index], set(args[index + 1 :])
    workflow, gate = args
    results = json.loads(os.environ["RESULTS"])

    if "--no-results" not in argv:
        failed = sorted(name for name, value in results.items() if value["result"] != "success")
        if failed:
            sys.exit("Jobs did not succeed: " + ", ".join(failed))

    with open(workflow_file(workflow), encoding="utf-8") as handle:
        jobs = job_ids(handle.read())
    if gate not in jobs:
        sys.exit(f"Cannot find {gate} in {workflow}; the gate check needs updating")
    missing = sorted(jobs - {gate} - exempt - set(results))
    if missing:
        sys.exit("Add these jobs to needs: " + ", ".join(missing))

    # On a pull request HEAD is the merge commit, whose first parent is the base
    # the merge was made against. A workflow new in this pull request has no base
    # copy, and a missing parent fails the check rather than skipping it.
    if (
        os.environ.get("GITHUB_EVENT_NAME") == "pull_request"
        and git("ls-tree", "--name-only", "HEAD^1", "--", workflow).strip()
    ):
        removed = job_ids(git("show", f"HEAD^1:{workflow}")) - jobs
        prefix = "removed-jobs:"
        body = os.environ.get("PR_BODY", "")
        declared = {
            line[len(prefix) :].strip() for line in body.splitlines() if line.startswith(prefix)
        }
        undeclared = sorted(job for job in removed if f"{workflow}#{job}" not in declared)
        if undeclared:
            sys.exit(
                "These jobs were removed from "
                + workflow
                + ": "
                + ", ".join(undeclared)
                + ". If that is intended, add a line 'removed-jobs: "
                + workflow
                + "#<job>' for each to the pull request description, then push again."
            )


if __name__ == "__main__":
    main(sys.argv[1:])
