"""Exercise workflow predecessor selection through isolated Python/Git fixtures."""

import contextlib
import hashlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
# The canonical checker lives in ryanduguid/release-policy, whose
# tests/test_repository_baseline.sh pins the same digest (SHA-256 after CRLF to LF).
POLICY_DIGEST = "8d48fa32c92d15bb476e28f2ad34400660f62ee337c527c1d9fd9b7f6230f62a"

NEEDS = "    needs: [changes, verify, secrets]\n"
SPEC = importlib.util.spec_from_file_location("check_gates", ROOT / ".github/ci/check_gates.py")
assert SPEC is not None and SPEC.loader is not None
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def workflow(jobs):
    return "name: fixture\n\non: pull_request\n\njobs:\n" + jobs


class GateReaderTests(unittest.TestCase):
    def names(self, jobs):
        parsed = checker.parse_jobs(workflow(jobs))
        self.assertIsNotNone(parsed)
        self.assertEqual(parsed[1], [])
        return parsed[0]

    def base(self, sources, head="  replacement:\n    name: protected\n"):
        def git(*args):
            if args[0] == "ls-tree":
                return "" if "-r" not in args else "\0".join(sources) + "\0"
            return sources[args[1].split(":", 1)[1]]
        with patch.object(checker, "git", side_effect=git), patch.object(checker, "base_workflow_paths", return_value=list(sources)):
            return checker.base_jobs(".github/workflows/new.yml", "replacement", workflow(head))

    def test_path_listing_preserves_linebreaks_and_rejects_invalid_utf8(self):
        paths = [".github/workflows/ci\r.yml", ".github/workflows/ci\n.yml"]
        completed = subprocess.CompletedProcess([], 0, "\0".join(paths).encode("utf-8") + b"\0")
        with patch.object(checker.subprocess, "run", return_value=completed) as run:
            self.assertEqual(checker.base_workflow_paths(), paths + [""])
            self.assertNotIn("encoding", run.call_args.kwargs)
            self.assertNotIn("text", run.call_args.kwargs)
        with patch.object(checker.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, b"\xff\0")):
            with self.assertRaises(UnicodeDecodeError):
                checker.base_workflow_paths()

    def test_literal_names_and_quoted_job_keys(self):
        self.assertEqual(self.names("  'plain':\n    runs-on: linux\n  other:\n    name: 'It''s quoted' # comment\n"), {"plain": {"plain"}, "other": {"It's quoted"}})

    def test_static_include_resolves_only_its_complete_rows(self):
        body = "  build:\n    name: ${{ matrix.package_dir }}\n    strategy:\n      matrix:\n        include:\n          - package_dir: first\n            import_name: first_import\n          - package_dir: 'second'\n            import_name: second_import\n"
        self.assertEqual(self.names(body)["build"], {"first", "second"})
        for replacement in ("            import_name: first_import", "          - package_dir: ${{ inputs.package }}", "          - package_dir: *alias", "          - package_dir: first\n            package_dir: second"):
            with self.subTest(replacement=replacement):
                names = self.names(body.replace("          - package_dir: first", replacement))["build"]
                self.assertTrue(checker.possible_context(names, "protected"))

    def test_extra_matrix_dimension_is_unknown(self):
        jobs = "  build:\n    name: ${{ matrix.package_dir }}\n    strategy:\n      matrix:\n        os: [linux]\n        include:\n          - package_dir: first\n"
        self.assertTrue(checker.possible_context(self.names(jobs)["build"], "protected"))

    def test_typed_yaml_matrix_scalars_are_not_misread_as_strings(self):
        for value in ("0x10", "1_000", "2026-10-15", ".nan", "YES", "on"):
            with self.subTest(value=value):
                jobs = "  build:\n    name: ${{ matrix.label }}\n    strategy:\n      matrix:\n        include:\n          - label: " + value + "\n"
                self.assertTrue(checker.possible_context(self.names(jobs)["build"], "protected"))

    def test_conditional_has_both_literal_outcomes(self):
        for condition in ("linux", "other"):
            names = self.names("  check:\n    name: ${{ matrix.os == '" + condition + "' && 'first' || 'second' }}\n")["check"]
            self.assertEqual(names, {"first", "second"})
            self.assertFalse(checker.possible_context(names, "protected"))

    def test_unknown_conditionals_do_not_disappear(self):
        for expression in ("matrix.os && '' || 'second'", "needs.build.outputs.name", "matrix.os == 'linux' && inputs.name || 'second'"):
            with self.subTest(expression=expression):
                names = self.names("  check:\n    name: ${{ " + expression + " }}\n")["check"]
                self.assertTrue(checker.possible_context(names, "protected"))

    def test_unknown_templates_keep_literal_ends(self):
        names = self.names("  check:\n    name: prefix (${{ inputs.name }})\n")["check"]
        self.assertFalse(checker.possible_context(names, "protected"))
        self.assertTrue(checker.possible_context(names, "prefix (protected)"))

    def test_expression_literal_ends_cannot_overlap(self):
        for pattern, context, expected in (
            ("new-${{ matrix.flavour }}-gates", "new-gates", False),
            ("new-${{ matrix.flavour }}gates", "new-gates", True),
            ("}}${{ inputs.context", "protected", True),
        ):
            with self.subTest(pattern=pattern):
                self.assertEqual(checker.possible_context({pattern}, context), expected)

    def test_tab_comments_and_blank_lines_do_not_end_the_jobs_block(self):
        for separator in ("\t# separator", "\t", " \t# separator"):
            with self.subTest(separator=separator):
                self.assertEqual(self.names("  replacement:\n" + separator + "\n    name: protected\n  critical:\n    runs-on: linux\n"), {"replacement": {"protected"}, "critical": {"critical"}})

    def test_implicit_matrix_names_keep_possible_suffixes(self):
        jobs = "  build:\n    strategy:\n      matrix:\n        os: [linux]\n"
        names = self.names(jobs)["build"]
        for context, expected in (("build (linux)", True), ("build", True), ("build-gates", False), ("protected", False)):
            with self.subTest(context=context):
                self.assertEqual(checker.possible_context(names, context), expected)
        for context in ("build", "build (linux)"):
            with self.subTest(context=context), self.assertRaisesRegex(SystemExit, "Cannot resolve"):
                self.base({".github/workflows/old.yml": workflow(jobs)}, f"  replacement:\n    name: {context}\n")
        self.assertEqual(self.base({".github/workflows/old.yml": workflow(jobs)}, "  replacement:\n    name: build-gates\n"), set())

    def test_reusable_jobs_keep_possible_called_job_names(self):
        names = self.names("  call:\n    name: component\n    uses: ./.github/workflows/package.yml\n")["call"]
        self.assertTrue(checker.possible_context(names, "component / protected"))
        self.assertFalse(checker.possible_context(names, "component"))
        with self.assertRaisesRegex(SystemExit, "Cannot resolve"):
            self.base({".github/workflows/old.yml": workflow("  call:\n    name: component\n    uses: ./.github/workflows/package.yml\n")}, "  replacement:\n    name: component / protected\n")

    def test_unknown_matching_base_job_fails_even_when_it_is_the_only_candidate(self):
        for name in ("${{ inputs.name }}", "*alias", "|\n      protected", "!tag protected"):
            with self.subTest(name=name), self.assertRaisesRegex(SystemExit, "Cannot resolve"):
                self.base({".github/workflows/old.yml": workflow("  old:\n    name: " + name + "\n")})

    def test_two_matching_jobs_in_one_workflow_are_ambiguous(self):
        with self.assertRaisesRegex(SystemExit, "More than one base job"):
            self.base({".github/workflows/old.yml": workflow("  first:\n    name: protected\n  second:\n    name: protected\n")})

    def test_duplicate_names_and_keys_are_not_treated_as_unique(self):
        for jobs in ("  old:\n    name: protected\n    name: other\n", "  old:\n    name: protected\n  old:\n    name: other\n", "  old: {name: protected}\n"):
            with self.subTest(jobs=jobs), self.assertRaises(SystemExit):
                self.base({".github/workflows/old.yml": workflow(jobs)})

    def test_unrelated_id_collision_does_not_override_reported_name(self):
        correct = workflow("  original:\n    name: protected\n")
        self.assertEqual(self.base({".github/workflows/other.yml": workflow("  replacement:\n    name: other\n"), ".github/workflows/old.yml": correct}), {"original"})

    def test_non_yaml_whitespace_names_stay_unknown(self):
        for text in ("protected\u00a0#gate", '"protected\u00a0#gate"', "protected\u2003"):
            with self.subTest(text=text):
                self.assertIsNone(checker.literal_name(text))
        with self.assertRaisesRegex(SystemExit, "Cannot determine"):
            self.base({}, "  replacement:\n    name: protected\u00a0#gate\n")

    def test_same_id_predecessor_binds_when_file_and_context_change(self):
        old = workflow("  replacement:\n    name: old-context\n  critical:\n    runs-on: linux\n")
        self.assertEqual(self.base({".github/workflows/old.yml": old}), {"replacement", "critical"})
        with self.assertRaisesRegex(SystemExit, "More than one base workflow"):
            self.base({".github/workflows/old.yml": old, ".github/workflows/two.yml": old})

    def test_same_id_gate_left_in_place_is_not_a_predecessor(self):
        old = workflow("  replacement:\n    name: old-context\n  critical:\n    runs-on: linux\n")
        path = ".github/workflows/old.yml"

        def git(*args):
            if args[0] == "ls-tree":
                return path if args[2] == "HEAD" and args[-1] == path else ""
            return old
        with patch.object(checker, "git", side_effect=git), patch.object(checker, "base_workflow_paths", return_value=[path]):
            self.assertEqual(checker.base_jobs(".github/workflows/new.yml", "replacement", workflow("  replacement:\n    name: new-context\n")), set())

    def test_unknown_head_name_and_unreadable_base_fail(self):
        with self.assertRaisesRegex(SystemExit, "Cannot determine"):
            self.base({}, "  replacement:\n    name: ${{ inputs.name }}\n")
        with self.assertRaisesRegex(SystemExit, "Cannot read base"):
            self.base({".github/workflows/old.yml": "jobs: *alias\n"})

    def test_same_path_and_context_sources_both_contribute_jobs(self):
        destination = workflow("  existing:\n    runs-on: linux\n")
        source = workflow("  original:\n    name: protected\n")
        with patch.object(checker, "git", side_effect=["new.yml", destination, source]), patch.object(checker, "base_workflow_paths", return_value=[".github/workflows/old.yml"]):
            self.assertEqual(checker.base_jobs(".github/workflows/new.yml", "replacement", workflow("  replacement:\n    name: protected\n")), {"existing", "original"})

    def test_event_forms_and_unknown_namespaces(self):
        for events, expected in (("pull_request", {"standalone"}), ("[push, workflow_call]", {"standalone", "callable"}), ("\n  workflow_call:\n    inputs: {}", {"callable"}), ("*alias", None), ("{workflow_call: {}}", None)):
            with self.subTest(events=events):
                self.assertEqual(checker.workflow_modes("name: fixture\non: " + events + "\njobs:\n  gate:\n"), expected)
        with self.assertRaisesRegex(SystemExit, "invocation namespace"):
            self.base({".github/workflows/old.yml": "name: fixture\non: *alias\njobs:\n  old:\n    name: protected\n"})

    def test_results_needs_exemptions_and_live_declarations(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "workflow.yml"
            path.write_text(workflow("  gate:\n    runs-on: linux\n  test:\n    runs-on: linux\n"), encoding="utf-8")
            with patch.object(checker, "workflow_file", return_value=str(path)), patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push", "RESULTS": '{"test":{"result":"failure"}}'}):
                with self.assertRaisesRegex(SystemExit, "Jobs did not succeed"):
                    checker.main(["workflow", "gate"])
                checker.main(["workflow", "gate", "--no-results"])
                with patch.dict(os.environ, {"RESULTS": "{}"}):
                    with self.assertRaisesRegex(SystemExit, "Add these jobs to needs"):
                        checker.main(["workflow", "gate"])
                    checker.main(["workflow", "gate", "--exempt", "test"])
                path.write_text("# removed-jobs: test\n" + path.read_text(encoding="utf-8"), encoding="utf-8")
                with self.assertRaisesRegex(SystemExit, "still exist"):
                    checker.main(["workflow", "gate", "--no-results"])

    def run_main(self, text, base=None, event="push", results='{"test":{"result":"success"}}'):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "workflow.yml"
            path.write_text(text, encoding="utf-8")
            with patch.object(checker, "workflow_file", return_value=str(path)), \
                    patch.object(checker, "base_jobs", return_value=base), \
                    patch.dict(os.environ, {"GITHUB_EVENT_NAME": event, "RESULTS": results}):
                checker.main(["workflow", "gate"])

    def test_declarations_count_only_at_the_start_of_a_line(self):
        jobs = "  gate:\n    runs-on: linux\n  test:\n    runs-on: linux\n"
        scalar = "    env:\n      NOTE: |\n        # removed-jobs: critical\n"
        nel = "    env:\n      NOTE: text\u0085# removed-jobs: critical\n"  # YAML 1.2 does not break lines at NEL
        for value in (scalar, nel):
            with self.subTest(value=value):
                # Value text naming a live job is not a stale declaration ...
                self.run_main(workflow("  gate:\n    runs-on: linux\n" + value + "  test:\n    runs-on: linux\n  critical:\n    runs-on: linux\n"),
                              results='{"test":{"result":"success"},"critical":{"result":"success"}}')
                # ... and never waives the removal of a base job.
                with self.assertRaisesRegex(SystemExit, "These jobs were removed"):
                    self.run_main(workflow("  gate:\n    runs-on: linux\n" + value + "  test:\n    runs-on: linux\n"),
                                  base={"gate", "test", "critical"}, event="pull_request")
        with self.assertRaisesRegex(SystemExit, "These jobs were removed"):
            self.run_main("  # removed-jobs: critical\n" + workflow(jobs), base={"gate", "test", "critical"}, event="pull_request")
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.run_main("# removed-jobs: critical\n" + workflow(jobs), base={"gate", "test", "critical"}, event="pull_request")
        self.assertEqual(output.getvalue(), "Declared removals from workflow: critical\n")

    def test_checker_is_the_release_policy_copy(self):
        text = (ROOT / ".github/ci/check_gates.py").read_bytes().replace(b"\r\n", b"\n")
        self.assertEqual(hashlib.sha256(text).hexdigest(), POLICY_DIGEST,
                         "change check_gates.py in ryanduguid/release-policy first, then copy it here unchanged")

    def test_configuration_errors_exit_with_one_line(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".github/workflows").mkdir(parents=True)
            (root / ".github/workflows/ci.yml").write_text(workflow("  gate:\n    runs-on: linux\n"), encoding="utf-8")
            env = {key: os.environ[key] for key in ("PATH", "SystemRoot", "TEMP", "TMP") if key in os.environ}
            env.update(GIT_CEILING_DIRECTORIES=str(root.parent), GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
            for args, extra, expected in (
                ([], {"RESULTS": "{}"}, "ValueError: expected WORKFLOW GATE [--exempt JOB ...] [--no-results]"),
                (["gate"], {}, "KeyError: 'RESULTS'"),
                (["gate"], {"RESULTS": "not JSON"}, "JSONDecodeError: Expecting value: line 1 column 1 (char 0)"),
                (["gate"], {"RESULTS": "[]"}, "AttributeError: 'list' object has no attribute 'items'"),
                (["gate"], {"RESULTS": "{}", "GITHUB_EVENT_NAME": "pull_request"}, "CalledProcessError: Command "),  # 3.15 drops the quotes around the list
            ):
                with self.subTest(expected=expected):
                    command = [sys.executable, "-I", str(ROOT / ".github/ci/check_gates.py")]
                    if args:
                        command += [".github/workflows/ci.yml", *args]
                    # Fixed checker argument list.
                    result = subprocess.run(command, cwd=root, env=env | extra,  # nosec B603  # nosemgrep
                                            capture_output=True, text=True, timeout=10)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertNotIn("Traceback", result.stderr)
                    self.assertTrue(result.stderr.startswith(expected), result.stderr)
                    self.assertEqual(result.stderr.count("\n"), 1, result.stderr)


def head_workflow(gate, name, declaration=""):
    head = (f"name: CI\n\non: pull_request\n\njobs:\n  {gate}:\n"
            f"    name: {name}\n    runs-on: ubuntu-latest\n"
            "    steps: [{run: 'true'}]\n")
    if declaration:
        head = f"# removed-jobs: {declaration}\n" + head
    return head


class GateCliTests(unittest.TestCase):
    IDENTITY = ("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "-c", "commit.gpgsign=false")

    def setUp(self):
        git_executable = shutil.which("git")
        assert git_executable is not None, "Git is required for the gate CLI regression"
        temporary = tempfile.TemporaryDirectory(prefix="gate-predecessor-")
        self.addCleanup(temporary.cleanup)
        outer = Path(temporary.name).resolve()
        self.assertEqual(outer.parent, Path(tempfile.gettempdir()).resolve())
        self.repo, home = outer / "repo", outer / "home"
        self.repo.mkdir()
        home.mkdir()
        self.env = {key: os.environ[key] for key in ("PATH", "SystemRoot", "TEMP", "TMP")
                    if key in os.environ}
        self.env.update(HOME=str(home), USERPROFILE=str(home), GIT_CONFIG_NOSYSTEM="1",
                        GIT_CONFIG_GLOBAL=os.devnull, GITHUB_EVENT_NAME="pull_request", RESULTS="{}")
        self.git_prefix = [rtk, "proxy", "git"] if (rtk := shutil.which("rtk")) else [git_executable]
        self.workflows = self.repo / ".github/workflows"
        self.workflows.mkdir(parents=True)
        source_workflows = ROOT / ".github/workflows"
        for source in sorted([*source_workflows.glob("*.yml"), *source_workflows.glob("*.yaml")]):
            (self.workflows / source.name).write_bytes(source.read_bytes())
        self.git_command("init", "--quiet", "--initial-branch=main")
        self.git_command("config", "core.quotePath", "true")

    def git_command(self, *args, raw=False):
        return subprocess.run([*self.git_prefix, *args], cwd=self.repo, env=self.env,
                              capture_output=True, text=not raw,
                              encoding=None if raw else "utf-8", check=True, timeout=10)

    def merge_change(self, base_name, base, filename, head):
        self.assertIn(f".github/workflows/{filename} replacement", head)
        self.git_command("add", "--all", "--", ".github/workflows")
        self.git_command(*self.IDENTITY, "commit", "--quiet", "-m", "Rename fixture")
        self.git_command("checkout", "--quiet", "main")
        (self.repo / ".github/first-parent-control.txt").write_text("base advances\n", encoding="utf-8")
        self.git_command("add", "--", ".github/first-parent-control.txt")
        self.git_command(*self.IDENTITY, "commit", "--quiet", "-m", "Advance base fixture")
        self.git_command(*self.IDENTITY, "merge", "--quiet", "--no-ff", "change", "-m", "Merge fixture")
        self.assertEqual(len(self.git_command("rev-list", "--parents", "-n", "1", "HEAD").stdout.split()), 3)
        self.assertEqual(self.git_command("show", f"HEAD^1:.github/workflows/{base_name}").stdout, base)
        self.assertEqual(self.git_command("show", f"HEAD^2:.github/workflows/{filename}").stdout, head)

    def test_all_aggregate_callers_accept_unchanged_workflows(self):
        self.git_command("add", "--", ".github/workflows")
        self.git_command(*self.IDENTITY, "commit", "--quiet", "-m", "Base fixture")
        self.git_command("checkout", "--quiet", "-b", "change")
        self.git_command(*self.IDENTITY, "commit", "--quiet", "--allow-empty", "-m", "Head fixture")
        self.git_command("checkout", "--quiet", "main")
        self.git_command(*self.IDENTITY, "commit", "--quiet", "--allow-empty", "-m", "Advance fixture")
        self.git_command(*self.IDENTITY, "merge", "--quiet", "--no-ff", "change", "-m", "Merge fixture")
        self.assertEqual(len(self.git_command("rev-list", "--parents", "-n", "1", "HEAD").stdout.split()), 3)
        for parent in ("HEAD^1", "HEAD^2"):
            self.assertEqual(self.git_command("rev-parse", f"{parent}^{{tree}}").stdout,
                             self.git_command("rev-parse", "HEAD^{tree}").stdout)
        for filename, gate, needs, flags in (
            ("verify.yml", "verify-gates", ("changes", "verify", "secrets"),
             ("--exempt", "codecov", "--exempt", "fuzzing")),
            ("codeql.yml", "codeql-gates", ("analyze",), ()),
        ):
            with self.subTest(workflow=filename):
                source = (ROOT / ".github/workflows" / filename).read_text(encoding="utf-8")
                self.assertIn(f"    needs: [{', '.join(needs)}]\n", source)
                self.assertIn(f".github/ci/check_gates.py .github/workflows/{filename} {gate}", source)
                result = subprocess.run([
                    sys.executable, "-I", str(ROOT / ".github/ci/check_gates.py"),
                    f".github/workflows/{filename}", gate, *flags,
                ], cwd=self.repo, env=self.env | {
                    "RESULTS": json.dumps({job: {"result": "success"} for job in needs}),
                }, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def run_cli_fixture(self, *, occupied=False, quoted_path=False, control_paths=False,
                        tab_comment=False, context="verify-gates", declaration="",
                        filename="renamed.yml"):
        base_name = "ci\r.yml" if control_paths else "c\u00ed.yml" if quoted_path else "verify.yml"
        base = (ROOT / ".github/workflows/verify.yml").read_text(encoding="utf-8")
        source_argument = ("$'.github/workflows/ci\\r.yml'" if control_paths
                           else f".github/workflows/{base_name}")
        if quoted_path or control_paths:
            base = base.replace(".github/workflows/verify.yml", source_argument)
            (self.workflows / "verify.yml").unlink()
            (self.workflows / base_name).write_text(base, encoding="utf-8")
        if control_paths:
            (self.workflows / "ci\n.yml").write_text(head_workflow("unrelated", "unrelated"), encoding="utf-8")
        if occupied:
            filename = "occupied.yml"
            (self.workflows / filename).write_text(head_workflow("replacement", "unrelated"), encoding="utf-8")
        self.git_command("add", "--", ".github/workflows")
        self.git_command(*self.IDENTITY, "commit", "--quiet", "-m", "Base fixture")
        self.git_command("checkout", "--quiet", "-b", "change")
        (self.workflows / base_name).unlink()
        aggregate = base.split("\n  verify-gates:\n", 1)[1]
        self.assertIn(NEEDS, aggregate)
        aggregate = aggregate.replace(NEEDS, "", 1).replace(
            f"{source_argument} verify-gates", f".github/workflows/{filename} replacement")
        head = ("name: CI\n\non: pull_request\n\njobs:\n  replacement:\n"
                f"    name: {context}\n" + aggregate)
        if tab_comment:
            head = head.replace("  replacement:\n", "  replacement:\n\t# separator\n", 1)
        if declaration:
            head = f"# removed-jobs: {declaration}\n" + head
        (self.workflows / filename).write_text(head, encoding="utf-8")
        self.merge_change(base_name, base, filename, head)
        changes = self.git_command("diff", "--no-renames", "--name-status", "-z", "HEAD^1", "HEAD", raw=True).stdout.decode("utf-8").split("\0")[:-1]
        expected = {("M", f".github/workflows/{filename}")} if filename == base_name else {
            ("D", f".github/workflows/{base_name}"),
            ("M" if occupied else "A", f".github/workflows/{filename}"),
        }
        self.assertEqual(set(zip(changes[::2], changes[1::2])), expected)
        if quoted_path:
            self.assertIn('".github/workflows/c\\303\\255.yml"', self.git_command(
                "ls-tree", "-r", "--name-only", "HEAD^1", "--", ".github/workflows/").stdout)
        if control_paths:
            paths = self.git_command("ls-tree", "-r", "--name-only", "-z", "HEAD^1", "--", ".github/workflows/", raw=True).stdout.decode("utf-8").split("\0")
            self.assertIn(".github/workflows/ci\r.yml", paths)
            self.assertIn(".github/workflows/ci\n.yml", paths)
        return subprocess.run([
            sys.executable, "-I", str(ROOT / ".github/ci/check_gates.py"),
            f".github/workflows/{filename}", "replacement",
            "--exempt", "codecov", "--exempt", "fuzzing",
        ], cwd=self.repo, env=self.env, capture_output=True, text=True, timeout=10)

    def test_cli_rejects_the_rename_in_a_real_git_parent(self):
        result = self.run_cli_fixture()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("These jobs were removed", result.stderr)
        self.assertIn("verify-gates", result.stderr)

    def test_cli_rejects_an_occupied_destination_bypass(self):
        result = self.run_cli_fixture(occupied=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("verify", result.stderr)

    def test_cli_retains_context_after_a_tab_comment(self):
        result = self.run_cli_fixture(tab_comment=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("These jobs were removed", result.stderr)

    def test_cli_retains_removals_from_a_git_quoted_path(self):
        result = self.run_cli_fixture(quoted_path=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("verify", result.stderr)

    def test_cli_accepts_declared_removals_from_a_git_quoted_path(self):
        result = self.run_cli_fixture(quoted_path=True,
            declaration="verify-gates, changes, codecov, fuzzing, secrets, verify")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_cli_accepts_an_unrelated_new_context(self):
        result = self.run_cli_fixture(quoted_path=True, context="new-gates")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_cli_deduplicates_an_existing_path(self):
        result = self.run_cli_fixture(filename="verify.yml",
            declaration="verify-gates, changes, codecov, fuzzing, secrets, verify")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    @unittest.skipIf(os.name == "nt", "CR/LF filenames require a POSIX working tree")
    def test_distinct_cr_lf_paths_retain_predecessor_removals(self):
        result = self.run_cli_fixture(control_paths=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("verify", result.stderr)

    @unittest.skipIf(os.name == "nt", "CR/LF filenames require a POSIX working tree")
    def test_distinct_cr_lf_paths_accept_declared_removals(self):
        result = self.run_cli_fixture(control_paths=True,
            declaration="verify-gates, changes, codecov, fuzzing, secrets, verify")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    @unittest.skipIf(os.name == "nt", "CR/LF filenames require a POSIX working tree")
    def test_distinct_cr_lf_paths_do_not_invent_a_predecessor(self):
        result = self.run_cli_fixture(control_paths=True, context="new-gates")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
