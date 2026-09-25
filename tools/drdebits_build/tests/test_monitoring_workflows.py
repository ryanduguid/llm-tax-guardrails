"""Exercise issue migration and recovery using the actual workflow shell steps."""
import json
import shlex
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[3]
BASH = shutil.which("bash")
JQ = shutil.which("jq")


@pytest.mark.skipif(not BASH or not JQ, reason="workflow integration needs Bash and jq")
@pytest.mark.parametrize("workflow,prefix,legacy", [
    ("link-check", "Link check", "SOURCE CURRENCY NOT CONFIRMED for listed links"),
    ("source-currency", "Source currency", "a pinned compilation has been superseded"),
])
@pytest.mark.parametrize("existing", ["legacy", "current", "both", "none"])
@pytest.mark.parametrize("recovery", [False, True])
@pytest.mark.parametrize("unrelated", [1, 101])
def test_monitoring_reuses_legacy_alerts_and_recovers_all_reports(
        tmp_path, workflow, prefix, legacy, existing, recovery, unrelated):
    issues = [{"number": 1000 + n, "title": prefix + ": unrelated issue"}
              for n in range(unrelated)]
    issues.append({"number": 98, "title": prefix + ": review required", "pull_request": {}})
    if existing in ("legacy", "both"):
        issues.append({"number": 11, "title": prefix + ": " + legacy})
    if existing in ("current", "both"):
        issues.append({"number": 12, "title": prefix + ": review required"})
    expected = sorted(issue["number"] for issue in issues if issue["number"] in (11, 12))
    document = yaml.safe_load((ROOT / f".github/workflows/{workflow}.yml").read_text())
    job = next(iter(document["jobs"].values()))
    name = "Record recovery" if recovery else "Open or update the report issue"
    script = next(step["run"] for step in job["steps"] if step.get("name") == name)
    # Mock only the GitHub transport. The workflow's real jq selection and shell
    # control flow run unchanged; no GitHub write or network request can occur.
    stub = r'''
gh() {
  case "$1 $2" in
    "api --paginate")
      [[ " $* " == *" --slurp "* ]] || return 91
      printf '%s' "$FIXTURE" | "$JQ" -r "${!#}"
      ;;
    "issue view") printf '%s\n' 'previous monitoring report' ;;
    *) printf '%s\n' "$*" >> operations.txt ;;
  esac
}
GITHUB_REPOSITORY=example/repository
CHECK_EXIT_CODE=2
'''
    pages = [issues[i:i + 100] for i in range(0, len(issues), 100)]
    setup = f"FIXTURE={shlex.quote(json.dumps(pages))}\nJQ={shlex.quote(Path(JQ).as_posix())}\n"
    (tmp_path / f"{workflow}.txt").write_text("synthetic finding\n")
    (tmp_path / "operations.txt").write_text("")
    path = tmp_path / "step.sh"
    path.write_text(setup + stub + script, encoding="utf-8", newline="\n")
    subprocess.run([BASH, "step.sh"], cwd=tmp_path, check=True, capture_output=True, text=True)
    operations = (tmp_path / "operations.txt").read_text().splitlines()
    if recovery:
        assert len(operations) == len(expected)
        assert all(f"issue close {number} " in operations[i] for i, number in enumerate(expected))
    elif expected:
        assert f"issue edit {expected[0]} " in operations[0]
        assert f"--title {prefix}: review required" in operations[0]
        assert len(operations) == len(expected)
        assert all(f"issue close {number} " in operations[i + 1]
                   for i, number in enumerate(expected[1:]))
    else:
        assert len(operations) == 1
        assert operations[0].startswith("issue create ")
    assert all("issue close 98 " not in operation for operation in operations)
