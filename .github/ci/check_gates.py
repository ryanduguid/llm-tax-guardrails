"""Check an aggregate gate job: its needs, their results and removed jobs.

usage: python .github/ci/check_gates.py WORKFLOW GATE [--exempt JOB ...] [--no-results]

Run from the repository root by the gate job of WORKFLOW, with RESULTS set to
toJSON(needs) and, on pull requests, the merge commit checked out with its
parents (fetch-depth: 2). It fails when:

- a needed job did not succeed (skipped with --no-results, where the gate
  applies its own result rules);
- a job in WORKFLOW other than GATE and the exempt jobs is missing from needs;
- on a pull request, a job on the base branch's copy of WORKFLOW (or, for a new
  or renamed file, of the one base workflow holding GATE) is gone, and WORKFLOW
  has no comment "# removed-jobs: JOB" naming it (the declaration lives in the
  file, so it cannot change without a new commit and a new run);
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


def parse_jobs(text: str) -> tuple[set[str], list[str]] | None:
    """Return a workflow's job ids and unreadable job-level lines, or None without a jobs block."""
    parts = text.split("\njobs:\n", 1)
    if len(parts) != 2:
        return None
    ids: set[str] = set()
    unreadable: list[str] = []
    for line in parts[1].splitlines():
        if line[:1] not in ("", " ", "#"):
            break  # the next top-level key ends the jobs block
        if line.startswith("  ") and line[2:3] not in ("", " ", "#"):
            name = job_key(line[2:])
            if name is None:
                unreadable.append(line)
            else:
                ids.add(name)
    return ids, unreadable


def job_ids(text: str) -> set[str]:
    """Return the job ids of a workflow, failing on any job-level line it cannot read."""
    parsed = parse_jobs(text)
    if parsed is None:
        sys.exit("Cannot find the jobs block; the gate check needs updating")
    ids, unreadable = parsed
    if unreadable:
        sys.exit(f"Cannot read the job key {unreadable[0]!r}; the gate check needs updating")
    return ids


def base_copy(workflow: str, gate: str) -> str | None:
    """Return the base branch's copy of WORKFLOW, or of its predecessor, or None if it is new."""
    if git("ls-tree", "--name-only", "HEAD^1", "--", workflow).strip():
        return git("show", f"HEAD^1:{workflow}")
    # Protection requires the gate by name, whatever file reports it, so a renamed
    # or replacement workflow is compared with the one base workflow holding GATE.
    candidates = []
    for path in git("ls-tree", "--name-only", "HEAD^1", "--", ".github/workflows/").splitlines():
        if path.endswith((".yml", ".yaml")):
            text = git("show", f"HEAD^1:{path}")
            parsed = parse_jobs(text)
            if parsed is not None and gate in parsed[0]:
                candidates.append(text)
    if len(candidates) > 1:
        sys.exit(
            f"More than one base workflow has a job named {gate}; the gate check needs updating"
        )
    return candidates[0] if candidates else None


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
        text = handle.read()
    jobs = job_ids(text)
    if gate not in jobs:
        sys.exit(f"Cannot find {gate} in {workflow}; the gate check needs updating")
    missing = sorted(jobs - {gate} - exempt - set(results))
    if missing:
        sys.exit("Add these jobs to needs: " + ", ".join(missing))

    # On a pull request HEAD is the merge commit, whose first parent is the base
    # the merge was made against. A missing parent fails the check rather than
    # skipping it; a workflow and gate both new in this pull request have no base.
    base = (
        base_copy(workflow, gate) if os.environ.get("GITHUB_EVENT_NAME") == "pull_request" else None
    )
    if base is not None:
        removed = job_ids(base) - jobs
        prefix = "# removed-jobs:"
        declared = set()
        for line in text.splitlines():
            if line.strip().startswith(prefix):
                declared.update(name.strip() for name in line.strip()[len(prefix) :].split(","))
        if removed & declared:
            print(f"Declared removals from {workflow}: {', '.join(sorted(removed & declared))}")
        undeclared = sorted(removed - declared)
        if undeclared:
            sys.exit(
                "These jobs were removed from "
                + workflow
                + ": "
                + ", ".join(undeclared)
                + ". If that is intended, add the comment '# removed-jobs: "
                + ", ".join(undeclared)
                + "' to that file."
            )


if __name__ == "__main__":
    main(sys.argv[1:])
