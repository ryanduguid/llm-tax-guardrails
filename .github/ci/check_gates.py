"""Check an aggregate gate job: its needs, their results and removed jobs.

usage: python .github/ci/check_gates.py WORKFLOW GATE [--exempt JOB ...] [--no-results]

Run from the repository root by the gate job of WORKFLOW, with RESULTS set to
toJSON(needs) and, on pull requests, the merge commit checked out with its
parents (fetch-depth: 2). It fails when:

- a needed job did not succeed (skipped with --no-results, where the gate
  applies its own result rules);
- a job in WORKFLOW other than GATE and the exempt jobs is missing from needs;
- on a pull request, a job in the base copy or unique context predecessor (or,
  without one, unique same-id predecessor) of WORKFLOW is gone, and WORKFLOW
  has no comment "# removed-jobs: JOB" naming it (the declaration lives in the
  file, so it cannot change without a new commit and a new run; it counts only
  at the start of a line, where it cannot be part of a YAML value);
- a line at job-key indentation in the jobs block is not a job key it can read.

Configuration errors (arguments, RESULTS, file reads, Git) exit with one line,
"ExceptionType: message", rather than a traceback.
"""

import json
import os
import re
import string
import subprocess
import sys

ID_START = set(string.ascii_letters + "_")
ID_CHARS = ID_START | set(string.digits + "-")
EXPRESSION_STRING = r"'((?:[^']|'')*)'"


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


def without_comment(text: str) -> str:
    """Drop a trailing comment (a # after whitespace) in one linear pass, then strip."""
    for index in range(1, len(text)):
        if text[index] == "#" and text[index - 1].isspace():
            return text[:index].strip()
    return text.strip()


def literal_name(text: str) -> str | None:
    """Read a single-line literal name; unsupported YAML stays unknown."""
    if any(char.isspace() and char not in " \t" for char in text):
        return None  # YAML separates with space and tab only, so other whitespace stays unknown
    text = text.strip()
    if text.startswith('"'):
        try:
            value, end = json.JSONDecoder().raw_decode(text)
        except ValueError:
            return None
        if isinstance(value, str) and value and not re.search(r"[\r\n]", value):
            tail = text[end:].strip()
            return value if not tail or tail.startswith("#") else None
        return None
    if text.startswith("'"):
        match = re.fullmatch(r"'((?:[^']|'')*)'\s*(?:#.*)?", text)
        return match[1].replace("''", "'") if match and match[1] else None
    text = without_comment(text)
    if (not text or text[0] in "!&*[]{}>|%@`#.+-0123456789" or re.search(r":\s|[\r\n]", text)
            or text.lower() in {"null", "true", "false", "yes", "no", "on", "off", "~"}
            or re.fullmatch(r"[-+]?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?", text)):
        return None
    return text


def block(lines: list[str], key: str, indent: int) -> list[str] | None:
    """Read one plain block property, retaining unsupported structures as unknown."""
    positions = [
        i for i, line in enumerate(lines)
        if line.startswith(" " * indent) and job_key(line[indent:]) == key
    ]
    if len(positions) != 1:
        return None
    start = positions[0]
    tail = lines[start].split(":", 1)[1].strip()
    if tail and not tail.startswith("#"):
        return None
    end = start + 1
    while end < len(lines):
        line = lines[end]
        if line.strip() and not line.lstrip().startswith("#"):
            if len(line) - len(line.lstrip(" ")) <= indent:
                break
        end += 1
    return lines[start + 1:end]


def matrix_values(lines: list[str], key: str) -> set[str] | None:
    """Read static include rows with scalar values and no other matrix dimensions."""
    strategy = block(lines, "strategy", 4)
    matrix = block(strategy, "matrix", 6) if strategy is not None else None
    include = block(matrix, "include", 8) if matrix is not None else None
    if include is None or matrix is None:
        return None
    for line in matrix:
        if line.strip() and not line.lstrip().startswith("#") and line[8:9] != " ":
            if job_key(line[8:]) != "include":
                return None
    rows: list[dict[str, str]] = []
    for line in include:
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if line.startswith("          - "):
            rows.append({})
        elif not line.startswith("            ") or line[12:13] in ("", " "):
            return None
        if not rows:
            return None
        field = job_key(line[12:])
        value = literal_name(line.split(":", 1)[1]) if field else None
        if field is None or field in rows[-1] or value is None or "${{" in value:
            return None
        rows[-1][field] = value
    return {row[key] for row in rows} if rows and all(key in row for row in rows) else None


def reported_names(name: str | None, lines: list[str], job: str) -> set[str] | None:
    """Resolve the finite expression forms used here; keep all others conservative."""
    if name is None:
        return None
    names = {name}
    variable = re.fullmatch(r"\$\{\{\s*matrix\.([A-Za-z_][A-Za-z_0-9]*)\s*\}\}", name)
    if variable:
        values = matrix_values(lines, variable[1])
        if values is not None:
            names = values
    conditional = re.fullmatch(
        r"\$\{\{\s*matrix\.[A-Za-z_][A-Za-z_0-9]*\s*==\s*" + EXPRESSION_STRING
        + r"\s*&&\s*" + EXPRESSION_STRING + r"\s*\|\|\s*" + EXPRESSION_STRING + r"\s*\}\}",
        name,
    )
    if conditional and conditional[2] and conditional[3]:
        names = {conditional[2].replace("''", "'"), conditional[3].replace("''", "'")}
    properties = {
        job_key(line[4:]) for line in lines
        if line.startswith("    ") and line[4:5] not in ("", " ")
    }
    if "name" not in properties and "strategy" in properties:
        # GitHub reports "job (values)", or the bare id when the strategy has no matrix.
        names = {job, job + " (${{ matrix }})"}
    if "uses" in properties:
        names = {name + " / ${{ called-job }}" for name in names}
    return names


def start_job(line: str, names: dict[str, str | None], bodies: dict[str, list[str]],
              unreadable: list[str]) -> tuple[str | None, bool]:
    """Record a job-key line; return its id (None if unreadable) and whether its value is unknown."""
    job = job_key(line[2:])
    if job is None:
        unreadable.append(line)
        return None, False
    if job in names:
        unreadable.append(line)
    tail = line.split(":", 1)[1].strip()
    unknown = bool(tail and not tail.startswith("#"))
    names[job] = None if unknown else job
    bodies[job] = []
    return job, unknown


def parse_jobs(text: str) -> tuple[dict[str, set[str] | None], list[str]] | None:
    """Return job ids mapped to possible reported names, plus unreadable job-level lines."""
    parts = text.split("\njobs:\n", 1)
    if len(parts) != 2:
        return None
    names: dict[str, str | None] = {}
    bodies: dict[str, list[str]] = {}
    unreadable: list[str] = []
    job = None
    unknown = False
    in_name = False
    has_property = False
    properties: set[str] = set()
    for line in parts[1].splitlines():
        if not line.strip() or line.lstrip(" \t").startswith("#"):
            continue
        if line[:1] not in ("", " ", "#"):
            break  # the next top-level key ends the jobs block
        if line.startswith("  ") and line[2:3] not in ("", " ", "#"):
            job, unknown = start_job(line, names, bodies, unreadable)
            in_name = has_property = False
            properties = set()
        elif job is not None and line.strip() and not line.lstrip().startswith("#"):
            bodies[job].append(line)
            indent = len(line) - len(line.lstrip(" "))
            if indent < 4 or (not has_property and indent != 4):
                unknown = True
            if line.startswith("    ") and line[4:5] not in ("", " "):
                has_property = True
                key = job_key(line[4:])
                in_name = key == "name"
                if key is None or key in properties:
                    unknown = True
                elif in_name:
                    names[job] = literal_name(line.split(":", 1)[1])
                if key is not None:
                    properties.add(key)
            elif in_name:
                unknown = True  # a continued name is outside the single-line reader
            if unknown:
                names[job] = None
    return {job: reported_names(name, bodies[job], job) for job, name in names.items()}, unreadable


def job_ids(text: str) -> set[str]:
    """Return the job ids of a workflow, failing on any job-level line it cannot read."""
    parsed = parse_jobs(text)
    if parsed is None:
        sys.exit("Cannot find the jobs block; the gate check needs updating")
    ids, unreadable = parsed
    if unreadable:
        sys.exit(f"Cannot read the job key {unreadable[0]!r}; the gate check needs updating")
    return set(ids)


def possible_context(names: set[str] | None, context: str) -> bool:
    """Keep unknown or expression-derived names unless their literal ends rule out a match."""
    if names is None:
        return True
    for name in names:
        if "${{" not in name:
            if name == context:
                return True
        else:
            prefix, expression = name.split("${{", 1)
            if "}}" not in expression:
                return True
            suffix = expression.rsplit("}}", 1)[1]
            if len(context) >= len(prefix) + len(suffix) and context.startswith(prefix) and context.endswith(suffix):
                return True
    return False


def workflow_modes(text: str) -> set[str] | None:
    """Read event names from a scalar, flat list or plain block; aliases stay unknown."""
    lines = text.splitlines()
    headers = [line for line in lines if job_key(line) == "on"]
    if len(headers) != 1:
        return None
    tail = headers[0].split(":", 1)[1].strip()
    if not tail or tail.startswith("#"):
        body = block(lines, "on", 0)
        if body is None:
            return None
        events: set[str] = set()
        for line in body:
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            if line.startswith("  ") and line[2:3] != " ":
                event = job_key(line[2:])
                if event is None or event in events:
                    return None
                events.add(event)
            elif not events:
                return None
    else:
        tail = without_comment(tail)
        values = tail[1:-1].split(",") if tail.startswith("[") and tail.endswith("]") else [tail]
        names = [literal_name(value) for value in values]
        if any(name is None or job_key(name + ":") != name for name in names):
            return None
        events = set(names)
    return {"callable" if event == "workflow_call" else "standalone" for event in events} or None


def gate_kept(path: str, gate: str) -> bool:
    """Whether HEAD still runs GATE from PATH, which then keeps its own obligations."""
    if not git("ls-tree", "--name-only", "HEAD", "--", path).strip():
        return False
    parsed = parse_jobs(git("show", f"HEAD:{path}"))
    return parsed is not None and not parsed[1] and gate in parsed[0]


def context_sources(path: str, jobs: dict[str, set[str] | None], base_modes: set[str] | None,
                    context: str) -> int:
    """Count the jobs in a base workflow that can report CONTEXT; unresolvable ones fail."""
    count = 0
    for names in jobs.values():
        if possible_context(names, context):
            if base_modes is None:
                sys.exit(f"Cannot determine the invocation namespace in {path}; update the reader")
            if names is None or any("${{" in name for name in names):
                sys.exit(f"Cannot resolve a possible gate context in {path}; update the reader")
            count += 1
    return count


def base_jobs(workflow: str, gate: str, head: str) -> set[str]:
    """Keep both destination and uniquely established context-source removal obligations."""
    required: set[str] = set()
    destination: set[str] = set()
    if git("ls-tree", "--name-only", "HEAD^1", "--", workflow).strip():
        destination = job_ids(git("show", f"HEAD^1:{workflow}"))
        required.update(destination)
    names = parse_jobs(head)[0][gate]
    if names is None or len(names) != 1 or any("${{" in name for name in names):
        sys.exit("Cannot determine the gate context; use a single-line literal name")
    context = next(iter(names))
    modes = workflow_modes(head)
    if modes is None or len(modes) != 1:
        sys.exit("Cannot determine the workflow invocation namespace; update the reader")
    # shortcut: only static include rows and literal-result conditionals resolve;
    # keep other expressions possible and use a maintained parser for broader YAML.
    candidates = []
    same_id = []
    for path in base_workflow_paths():
        # GitHub reads workflows only from the top level of .github/workflows.
        if path.count("/") == 2 and path.endswith((".yml", ".yaml")):
            text = git("show", f"HEAD^1:{path}")
            base_modes = workflow_modes(text)
            if base_modes is not None and not modes & base_modes:
                continue
            parsed = parse_jobs(text)
            if parsed is None or not parsed[0] or parsed[1]:
                sys.exit(f"Cannot read base workflow {path}; the gate check needs updating")
            if gate in parsed[0] and path != workflow and not gate_kept(path, gate):
                same_id.append(set(parsed[0]))
            candidates += [set(parsed[0])] * context_sources(path, parsed[0], base_modes, context)
    if len(candidates) > 1:
        sys.exit(
            f"More than one base job can precede {gate}; the gate check needs updating"
        )
    if candidates:
        required.update(candidates[0])
    elif gate not in destination:
        # A gate that kept its id but changed file and context still owes the base jobs.
        if len(same_id) > 1:
            sys.exit(f"More than one base workflow can precede {gate}; the gate check needs updating")
        if same_id:
            required.update(same_id[0])
    return required


def workflow_file(argument: str) -> str:
    """Return the path of WORKFLOW, which must name a file directly in .github/workflows."""
    # The path comes from the directory listing, never from the argument itself.
    prefix = ".github/workflows/"
    listing = {entry.name: entry.path for entry in os.scandir(prefix) if entry.is_file()}
    name = argument[len(prefix) :] if argument.startswith(prefix) else ""
    if name not in listing:
        sys.exit(f"{argument} is not a file in .github/workflows")
    return listing[name]


def base_workflow_paths() -> list[str]:
    """Preserve pathname bytes, including CR and LF, before strict UTF-8 decoding."""
    listing = subprocess.run(
        ["git", "ls-tree", "-r", "--name-only", "-z", "HEAD^1", "--", ".github/workflows/"],
        capture_output=True, check=True,
    ).stdout
    return listing.decode("utf-8").split("\0")


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8", check=True).stdout  # nosec B607


def main(argv: list[str]) -> None:
    args = [a for a in argv if a != "--no-results"]
    exempt: set[str] = set()
    if "--exempt" in args:
        index = args.index("--exempt")
        args, exempt = args[:index], set(args[index + 1 :])
    if len(args) != 2:
        raise ValueError("expected WORKFLOW GATE [--exempt JOB ...] [--no-results]")
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

    prefix = "# removed-jobs:"
    declared: set[str] = set()
    # Only "\n" ends a YAML line; splitlines() would also split a value at NEL or U+2028.
    for line in text.split("\n"):
        if line.startswith(prefix):
            declared.update(name.strip() for name in line[len(prefix) :].split(","))
    # A declaration naming a job that still exists would waive a later removal.
    live = sorted(declared & jobs)
    if live:
        sys.exit("These removed-jobs declarations name jobs that still exist: " + ", ".join(live))

    # On a pull request HEAD is the merge commit, whose first parent is the base
    # the merge was made against. A missing parent fails the check rather than
    # skipping it; a workflow and gate both new in this pull request have no base.
    base = (
        base_jobs(workflow, gate, text) if os.environ.get("GITHUB_EVENT_NAME") == "pull_request" else None
    )
    if base is not None:
        removed = base - jobs
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
    try:
        main(sys.argv[1:])
    except (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.CalledProcessError) as error:
        sys.exit(f"{type(error).__name__}: {error}")
