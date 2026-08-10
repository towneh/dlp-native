"""Shared fixtures for the build-script tests.

scripts/ is not a package, so the script under test is loaded straight from its
path. Importing it is side-effect free: everything lives behind main().
"""

import importlib.util
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = REPO_ROOT / "scripts" / "stage_stdlib.py"


@pytest.fixture(scope="session")
def stage_stdlib():
    spec = importlib.util.spec_from_file_location("stage_stdlib", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture
def out_dir(tmp_path):
    """Where the tests send archives, so nothing lands in the real package."""
    return tmp_path / "out"


@pytest.fixture
def run_stage(stage_stdlib, out_dir, monkeypatch):
    """Run main() with argv; return the zip it wrote.

    --out-dir is always passed: the default is anchored to the repo, and tests
    must not write into unity_package/.
    """

    def _run(*argv):
        monkeypatch.setattr(sys, "argv", ["stage_stdlib.py", *argv, "--out-dir", str(out_dir)])
        stage_stdlib.main()
        return out_dir / f"{argv[0]}.zip"

    return _run


@pytest.fixture
def prefix(tmp_path):
    """An empty Python prefix to populate per test."""
    root = tmp_path / "prefix"
    root.mkdir()
    return root
