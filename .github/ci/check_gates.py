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
import re
import subprocess
import sys

KEY = re.compile(r"""(["']?)([A-Za-z_][\w-]*)\1[ \t]*:(?:[ \t]|$)""")


def job_ids(text: str) -> set[str]:
    """Return the job ids of a workflow, failing on any job-level line it cannot read."""
    parts = text.split("\njobs:\n", 1)
    if len(parts) != 2:
        sys.exit("Cannot find the jobs block; the gate check needs updating")
    # The block ends at the next top-level key; column-0 comments stay inside it.
    block = re.split(r"(?m)^(?=[^\s#])", parts[1], maxsplit=1)[0]
    ids = set()
    for line in block.splitlines():
        if line.startswith("  ") and line[2:3] not in ("", " ", "#"):
            match = KEY.match(line[2:])
            if match is None:
                sys.exit(f"Cannot read the job key {line!r}; the gate check needs updating")
            ids.add(match[2])
    return ids


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def main(argv: list[str]) -> None:
    args = [a for a in argv if a != "--no-results"]
    exempt: set[str] = set()
    if "--exempt" in args:
        index = args.index("--exempt")
        args, exempt = args[:index], set(args[index + 1:])
    workflow, gate = args
    results = json.loads(os.environ["RESULTS"])

    if "--no-results" not in argv:
        failed = sorted(name for name, value in results.items() if value["result"] != "success")
        if failed:
            sys.exit("Jobs did not succeed: " + ", ".join(failed))

    with open(workflow, encoding="utf-8") as handle:
        jobs = job_ids(handle.read())
    if gate not in jobs:
        sys.exit(f"Cannot find {gate} in {workflow}; the gate check needs updating")
    missing = sorted(jobs - {gate} - exempt - set(results))
    if missing:
        sys.exit("Add these jobs to needs: " + ", ".join(missing))

    # On a pull request HEAD is the merge commit, whose first parent is the base
    # the merge was made against. A workflow new in this pull request has no base
    # copy, and a missing parent fails the check rather than skipping it.
    if os.environ.get("GITHUB_EVENT_NAME") == "pull_request" and git(
        "ls-tree", "--name-only", "HEAD^1", "--", workflow
    ).strip():
        removed = job_ids(git("show", f"HEAD^1:{workflow}")) - jobs
        body = os.environ.get("PR_BODY", "")
        declared = set(re.findall(r"(?m)^removed-jobs:[ \t]*(\S+)[ \t]*$", body))
        undeclared = sorted(job for job in removed if f"{workflow}#{job}" not in declared)
        if undeclared:
            sys.exit(
                "These jobs were removed from " + workflow + ": " + ", ".join(undeclared)
                + ". If that is intended, add a line 'removed-jobs: " + workflow
                + "#<job>' for each to the pull request description, then push again."
            )


if __name__ == "__main__":
    main(sys.argv[1:])
