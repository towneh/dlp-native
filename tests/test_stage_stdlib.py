"""Behaviour of the stdlib staging script shipped at unity_package/Python~/.

The base auto-detection and the exclusion set decide what ships inside every
platform's stdlib zip, and a wrong choice there surfaces as a runtime failure in
Unity rather than a build error. These pin the parts that are easy to get subtly
wrong: numeric version ranking, the directory requirement, and which trees are
dropped.
"""

import zipfile
from pathlib import Path

import pytest


def write(path: Path, text: str = "x") -> Path:
    """Create a file and any missing parents."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def names(zip_path):
    with zipfile.ZipFile(zip_path) as z:
        return set(z.namelist())


# ── POSIX base auto-detection ────────────────────────────────────────────────


def test_ranks_versions_numerically_not_lexically(run_stage, prefix):
    for minor in ("9", "10", "12", "14"):
        write(prefix / "lib" / f"python3.{minor}" / "marker.py", minor)

    out = run_stage("linux-x86_64", "--prefix", str(prefix))

    # Sorting the names would pick python3.9, which sorts last as a string.
    assert names(out) == {"lib/python3.14/marker.py"}


def test_ignores_matching_names_that_are_not_directories(run_stage, prefix):
    write(prefix / "lib" / "python3.14" / "marker.py")
    write(prefix / "lib" / "python3.99")  # a plain file, not a stdlib directory

    out = run_stage("linux-x86_64", "--prefix", str(prefix))

    assert names(out) == {"lib/python3.14/marker.py"}


def test_single_version_prefix(run_stage, prefix):
    write(prefix / "lib" / "python3.14" / "os.py")

    out = run_stage("linux-x86_64", "--prefix", str(prefix))

    assert names(out) == {"lib/python3.14/os.py"}


def test_exits_when_lib_is_missing(run_stage, prefix):
    with pytest.raises(SystemExit) as exc:
        run_stage("linux-x86_64", "--prefix", str(prefix))

    assert "lib/ not found" in str(exc.value)


def test_exits_when_no_python_directory_present(run_stage, prefix):
    (prefix / "lib").mkdir()
    write(prefix / "lib" / "python3.99")  # only a stray file

    with pytest.raises(SystemExit) as exc:
        run_stage("linux-x86_64", "--prefix", str(prefix))

    assert "no python3.x directory" in str(exc.value)


# ── Base selection on other platforms ────────────────────────────────────────


def test_windows_uses_lib_and_dlls(run_stage, prefix):
    # No lib/pythonX.Y here on purpose: had the POSIX branch run instead, it would
    # have exited rather than staged anything. Deliberately not creating a lowercase
    # lib/ alongside Lib/ — that collides on a case-insensitive filesystem and makes
    # the test host-dependent.
    write(prefix / "Lib" / "os.py")
    write(prefix / "DLLs" / "unicodedata.pyd")

    out = run_stage("windows-x86_64", "--prefix", str(prefix))

    assert names(out) == {"Lib/os.py", "DLLs/unicodedata.pyd"}


def test_explicit_bases_override_detection(run_stage, prefix):
    write(prefix / "custom" / "mod.py")
    write(prefix / "lib" / "python3.14" / "ignored.py")

    out = run_stage("linux-x86_64", "--prefix", str(prefix), "--bases", "custom")

    assert names(out) == {"custom/mod.py"}


def test_missing_base_warns_but_still_stages_the_rest(run_stage, prefix, capsys):
    write(prefix / "Lib" / "os.py")

    out = run_stage("windows-x86_64", "--prefix", str(prefix), "--bases", "Lib", "Absent")

    assert names(out) == {"Lib/os.py"}
    assert "Absent" in capsys.readouterr().err


# ── Exclusions ───────────────────────────────────────────────────────────────


@pytest.mark.parametrize(
    "excluded",
    [
        "__pycache__",
        "ensurepip",
        "idlelib",
        "lib2to3",
        "pydoc_data",
        "site-packages",
        "test",
        "tkinter",
        "turtledemo",
    ],
)
def test_always_excluded_trees_are_dropped(run_stage, prefix, excluded):
    stdlib = prefix / "lib" / "python3.14"
    write(stdlib / "os.py")
    write(stdlib / excluded / "dropped.py")

    out = run_stage("linux-x86_64", "--prefix", str(prefix))

    assert names(out) == {"lib/python3.14/os.py"}


def test_exclusions_apply_at_any_depth(run_stage, prefix):
    stdlib = prefix / "lib" / "python3.14"
    write(stdlib / "os.py")
    write(stdlib / "unittest" / "test" / "dropped.py")

    out = run_stage("linux-x86_64", "--prefix", str(prefix))

    assert names(out) == {"lib/python3.14/os.py"}


def test_extra_excludes_layer_on_top_of_the_builtin_set(run_stage, prefix):
    stdlib = prefix / "lib" / "python3.14"
    write(stdlib / "os.py")
    write(stdlib / "lib-dynload" / "dropped.so")
    write(stdlib / "idlelib" / "also-dropped.py")

    out = run_stage("linux-x86_64", "--prefix", str(prefix), "--exclude-dirs", "lib-dynload")

    assert names(out) == {"lib/python3.14/os.py"}


# ── Archive shape ────────────────────────────────────────────────────────────


def test_archive_paths_are_prefix_relative_and_posix(run_stage, prefix):
    write(prefix / "lib" / "python3.14" / "json" / "decoder.py")

    out = run_stage("linux-x86_64", "--prefix", str(prefix))

    entry = next(iter(names(out)))
    assert entry == "lib/python3.14/json/decoder.py"
    assert "\\" not in entry


def test_entries_are_stored_uncompressed(run_stage, prefix):
    write(prefix / "lib" / "python3.14" / "os.py", "x" * 4096)

    out = run_stage("linux-x86_64", "--prefix", str(prefix))

    with zipfile.ZipFile(out) as z:
        assert all(i.compress_type == zipfile.ZIP_STORED for i in z.infolist())


def test_output_is_named_after_the_platform(run_stage, prefix):
    write(prefix / "lib" / "python3.14" / "os.py")

    out = run_stage("ios-arm64", "--prefix", str(prefix))

    assert out.name == "ios-arm64.zip"
    assert out.is_file()


# ── Android trust store ──────────────────────────────────────────────────────


def test_android_bundles_the_trust_store(run_stage, prefix):
    write(prefix / "lib" / "python3.14" / "os.py")
    write(prefix / "etc" / "tls" / "cert.pem")

    out = run_stage("android-arm64-v8a", "--prefix", str(prefix))

    assert names(out) == {"lib/python3.14/os.py", "etc/tls/cert.pem"}


def test_android_without_a_trust_store_is_an_error(run_stage, prefix, out_dir):
    write(prefix / "lib" / "python3.14" / "os.py")

    with pytest.raises(SystemExit) as exc:
        run_stage("android-arm64-v8a", "--prefix", str(prefix))

    assert "etc/tls" in str(exc.value)
    assert not (out_dir / "android-arm64-v8a.zip").exists()


# ── Failure to stage anything ────────────────────────────────────────────────


def test_staging_nothing_is_an_error(run_stage, prefix, out_dir):
    # A base that exists but holds only excluded content: the walk finds the
    # directory, so the missing-base warning never fires, yet nothing is written.
    write(prefix / "lib" / "python3.14" / "test" / "only-excluded.py")

    with pytest.raises(SystemExit) as exc:
        run_stage("linux-x86_64", "--prefix", str(prefix))

    assert "staged nothing" in str(exc.value)
    # An empty archive left behind would satisfy DlpBuildPreprocessor's
    # "already staged" check and ship a stdlib-less zip.
    assert not (out_dir / "linux-x86_64.zip").exists()


def test_empty_base_is_an_error_and_leaves_no_archive(run_stage, prefix, out_dir):
    write(prefix / "Lib" / "placeholder" / "x.py")

    with pytest.raises(SystemExit) as exc:
        run_stage("windows-x86_64", "--prefix", str(prefix), "--bases", "Absent")

    assert "staged nothing" in str(exc.value)
    assert not (out_dir / "windows-x86_64.zip").exists()


# ── Output location ──────────────────────────────────────────────────────────


def test_default_out_dir_is_anchored_to_the_package_not_the_cwd(
    stage_stdlib, tmp_path, monkeypatch
):
    monkeypatch.chdir(tmp_path)
    default = Path(stage_stdlib.DEFAULT_OUT_DIR)

    assert default.is_absolute()
    assert default.parts[-4:] == ("unity_package", "StreamingAssets", "dlp", "stdlib")
    # Anchored to the script, so the cwd above must not appear in it.
    assert tmp_path not in default.parents
    # parents[2] is the package root, which is what the script derives from. A
    # consumer only has unity_package/ on disk, so this must not reach the repo.
    assert (default.parents[2] / "Python~" / "stage_stdlib.py").is_file()
