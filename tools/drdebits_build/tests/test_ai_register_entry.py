"""The supplier information for a firm's AI register names the current release."""

from __future__ import annotations

import re
from pathlib import Path

from drdebits_build.build import find_root
from drdebits_build.model import load_metadata

ROOT = find_root(Path(__file__).resolve())


def test_register_entry_names_the_release_tag_in_metadata():
    # docs/ai-register-entry.md is supplier information a firm uses in its AI
    # register, so the release it describes must move with the release tag.
    tag = load_metadata(ROOT / "src" / "data" / "metadata.yaml")["release_tag"]
    entry = (ROOT / "docs" / "ai-register-entry.md").read_text(encoding="utf-8")

    assert f"| Name and version | DrDebits (`llm-tax-guardrails`) release {tag}, " in entry
    # The limits and updates rows name the release too; a bump must revisit them.
    assert set(re.findall(r"\bv\d+\.\d+\.\d+\b", entry)) == {tag}


def test_readme_lists_the_register_entry():
    readme = (ROOT / "README.md").read_text(encoding="utf-8")

    assert "(./docs/ai-register-entry.md)" in readme
