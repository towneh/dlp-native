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
def run_stage(stage_stdlib, tmp_path, monkeypatch):
    """Run main() with argv in a scratch cwd; return the zip it wrote.

    The script resolves its output relative to the working directory, so the
    chdir is what puts the archive somewhere disposable.
    """

    def _run(*argv):
        monkeypatch.chdir(tmp_path)
        monkeypatch.setattr(sys, "argv", ["stage_stdlib.py", *argv])
        stage_stdlib.main()
        return tmp_path / "unity_package/StreamingAssets/dlp/stdlib" / f"{argv[0]}.zip"

    return _run


@pytest.fixture
def prefix(tmp_path):
    """An empty Python prefix to populate per test."""
    root = tmp_path / "prefix"
    root.mkdir()
    return root
