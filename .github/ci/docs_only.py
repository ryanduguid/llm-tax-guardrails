"""Skip a workflow's jobs on documentation-only pull requests, and check the gate.

usage: python3 .github/ci/docs_only.py select [--checked PREFIX ...]
       python3 .github/ci/docs_only.py gate

select runs in the workflow's changes job on the pull request merge commit,
checked out with its parents (fetch-depth: 2), and prints docs-only=true or
docs-only=false for $GITHUB_OUTPUT. A pull request is documentation-only when
every changed path is a *.md file, under docs/, or named llms.txt or
CITATION.cff, and no check reads any of them: a changed path under a --checked
prefix, or whose path or file name appears in a tracked file that is not
documentation, runs every job. Any other event, a commit that is not a
two-parent merge or a failed git call also runs every job.

gate runs in the aggregate gate job with RESULTS set to toJSON(needs). The
changes job must succeed. Every other job must succeed, or on a
documentation-only pull request may be skipped.
"""

from __future__ import annotations

import json
import os
import posixpath
import subprocess
import sys

DOCS_EXCLUDES = [
    ":(exclude,glob)**/*.md",
    ":(exclude)docs",
    ":(exclude,glob)**/llms.txt",
    ":(exclude,glob)**/CITATION.cff",
]


def is_doc(path: str) -> bool:
    name = posixpath.basename(path)
    return path.endswith(".md") or path.startswith("docs/") or name in ("llms.txt", "CITATION.cff")


def git(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=False)


def referenced(path: str) -> bool:
    """Whether a tracked file other than documentation names the file (its path names it too).

    A failed search counts as a reference, so every job runs.
    """
    found = git("grep", "-l", "-F", "-e", posixpath.basename(path), "--", ".", *DOCS_EXCLUDES)
    return found.returncode != 1


def docs_only(checked: list[str]) -> tuple[bool, str]:
    if os.environ.get("GITHUB_EVENT_NAME") != "pull_request":
        return False, "not a pull request"
    head = git("cat-file", "-p", "HEAD")
    parents = [line for line in head.stdout.split("\n\n", 1)[0].splitlines() if line.startswith("parent ")]
    if head.returncode or len(parents) != 2:
        return False, "HEAD is not a pull request merge commit"
    diff = git("diff", "--name-only", "--no-renames", "-z", "HEAD^1", "HEAD", "--")
    paths = [path for path in diff.stdout.split("\0") if path]
    if diff.returncode or not paths:
        return False, "cannot list the changed paths"
    for path in paths:
        if not is_doc(path):
            return False, f"{path} is not documentation"
        if path.startswith(tuple(checked)):
            return False, f"checks read {path}"
        if referenced(path):
            return False, f"a tracked file names {path}"
    return True, f"{len(paths)} documentation file(s) that no check reads"


def check_results(results: object) -> None:
    if not isinstance(results, dict):
        raise ValueError("Job results must be an object")
    changes = results.get("changes")
    if not isinstance(changes, dict) or changes.get("result") != "success":
        raise ValueError("The changes job did not succeed")
    outputs = changes.get("outputs")
    selected = outputs.get("docs-only") if isinstance(outputs, dict) else None
    if selected not in ("true", "false"):
        raise ValueError("The changes job gave no docs-only selection")
    if selected == "true" and os.environ.get("GITHUB_EVENT_NAME") != "pull_request":
        raise ValueError("Only a pull request can skip jobs")
    allowed = {"success", "skipped"} if selected == "true" else {"success"}
    failed = sorted(
        name for name, job in results.items()
        if not isinstance(job, dict) or job.get("result") not in allowed
    )
    if failed:
        raise ValueError("Jobs did not succeed: " + ", ".join(failed))


def main(argv: list[str]) -> int:
    try:
        if argv[1:2] == ["select"] and len(argv) % 2 == 0 and all(a == "--checked" for a in argv[2::2]):
            selected, reason = docs_only(argv[3::2])
            print(f"Documentation-only: {selected} ({reason})", file=sys.stderr)
            print(f"docs-only={'true' if selected else 'false'}")
        elif argv[1:] == ["gate"]:
            check_results(json.loads(os.environ.get("RESULTS", "")))
        else:
            print(__doc__.split("\n\n")[1], file=sys.stderr)
            return 2
    except (OSError, ValueError, TypeError) as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
