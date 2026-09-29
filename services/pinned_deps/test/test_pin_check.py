"""Tests for dependency pinning and lockfile-drift rejection (#501)."""
from __future__ import annotations

import re
import subprocess
import tomllib
from pathlib import Path

import pytest

from services.pinned_deps.pin_check import (
    check,
    check_lockfile,
    check_requirements,
    load_lockfile,
    load_requirements,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
CONTRACTS = REPO_ROOT / "contracts"
SCRIPT = REPO_ROOT / "scripts" / "reproducible-build.sh"
DOCKERFILE = REPO_ROOT / "docker" / "Dockerfile.canonical-build"
TOOLCHAIN = REPO_ROOT / "rust-toolchain.toml"


def _manifest(root: Path, body: str) -> None:
    (root / "Cargo.toml").write_text(body, encoding="utf-8")


def _member(root: Path, deps: str) -> None:
    member = root / "contracts" / "c"
    member.mkdir(parents=True)
    (member / "Cargo.toml").write_text(
        f'[package]\nname = "c"\nversion = "0.1.0"\nedition = "2021"\n\n{deps}',
        encoding="utf-8",
    )


def _lock(root: Path, body: str) -> None:
    (root / "Cargo.lock").write_text(body, encoding="utf-8")


# -- the real repository is pinned ------------------------------------------


def test_repository_dependencies_are_fully_pinned():
    assert check(REPO_ROOT) == []


def test_every_workspace_requirement_is_exact():
    requirements = load_requirements(REPO_ROOT)
    assert requirements, "no workspace dependencies found"
    for req in requirements:
        if req.is_local:
            continue
        assert req.version and req.version.startswith("="), req
        assert re.match(r"^=\d+\.\d+\.\d+", req.version), req


def test_lockfile_is_committed_and_hashed():
    lock = REPO_ROOT / "Cargo.lock"
    assert lock.exists()
    data = tomllib.loads(lock.read_text(encoding="utf-8"))
    registry = [
        p
        for p in data["package"]
        if str(p.get("source", "")).startswith("registry+")
    ]
    assert registry
    for package in registry:
        assert package.get("checksum"), package["name"]


def test_pins_match_the_locked_versions():
    requirements = load_requirements(REPO_ROOT)
    assert check_lockfile(requirements, REPO_ROOT) == []


# -- floating requirements are rejected -------------------------------------


@pytest.mark.parametrize("version", ["*", "1", "1.5", "^1.5.0", "~1.5.0", ">=1.5, <2"])
def test_floating_requirements_are_rejected(tmp_path: Path, version: str):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c"]\n')
    _member(tmp_path, f'[dependencies]\nserde = {{ version = "{version}" }}\n')
    errors = check_requirements(load_requirements(tmp_path))
    assert any("not an exact pin" in e for e in errors), errors


def test_exact_requirement_is_accepted(tmp_path: Path):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c"]\n')
    _member(tmp_path, '[dependencies]\nserde = { version = "=1.0.228" }\n')
    assert check_requirements(load_requirements(tmp_path)) == []


def test_bare_string_requirement_is_accepted_when_exact(tmp_path: Path):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c"]\n')
    _member(tmp_path, '[dependencies]\nserde = "=1.0.228"\n')
    assert check_requirements(load_requirements(tmp_path)) == []


def test_git_dependency_must_pin_a_rev(tmp_path: Path):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c"]\n')
    _member(
        tmp_path,
        '[dependencies]\nsome-crate = { git = "https://example.invalid/x", branch = "main" }\n',
    )
    errors = check_requirements(load_requirements(tmp_path))
    assert any("immutable `rev`" in e for e in errors)


def test_git_dependency_with_rev_is_accepted(tmp_path: Path):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c"]\n')
    _member(
        tmp_path,
        '[dependencies]\nsome-crate = { git = "https://example.invalid/x", rev = "abc123" }\n',
    )
    assert check_requirements(load_requirements(tmp_path)) == []


def test_workspace_path_dependency_is_not_flagged(tmp_path: Path):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c", "contracts/d"]\n')
    _member(tmp_path, '[dependencies]\nd = { path = "../d" }\n')
    dep = tmp_path / "contracts" / "d"
    dep.mkdir(parents=True)
    (dep / "Cargo.toml").write_text(
        '[package]\nname = "d"\nversion = "0.1.0"\nedition = "2021"\n', encoding="utf-8"
    )
    assert check_requirements(load_requirements(tmp_path)) == []


# -- lockfile drift is rejected ---------------------------------------------


def test_missing_lockfile_is_rejected(tmp_path: Path):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c"]\n')
    _member(tmp_path, '[dependencies]\nserde = { version = "=1.0.228" }\n')
    errors = check(tmp_path)
    assert any("Cargo.lock is missing" in e for e in errors)


def test_lock_without_checksum_is_rejected(tmp_path: Path):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c"]\n')
    _member(tmp_path, '[dependencies]\nserde = { version = "=1.0.228" }\n')
    _lock(
        tmp_path,
        'version = 4\n\n[[package]]\nname = "serde"\nversion = "1.0.228"\n'
        'source = "registry+https://github.com/rust-lang/crates.io-index"\n',
    )
    errors = check(tmp_path)
    assert any("no checksum" in e for e in errors)


def test_pin_that_disagrees_with_the_lock_is_drift(tmp_path: Path):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c"]\n')
    _member(tmp_path, '[dependencies]\nserde = { version = "=1.0.999" }\n')
    _lock(
        tmp_path,
        'version = 4\n\n[[package]]\nname = "serde"\nversion = "1.0.228"\n'
        'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
        'checksum = "deadbeef"\n',
    )
    errors = check(tmp_path)
    assert any("dependency drift" in e for e in errors)


def test_stale_lock_missing_a_direct_dependency_is_rejected(tmp_path: Path):
    _manifest(tmp_path, '[workspace]\nmembers = ["contracts/c"]\n')
    _member(tmp_path, '[dependencies]\nserde = { version = "=1.0.228" }\n')
    _lock(tmp_path, "version = 4\n")
    errors = check(tmp_path)
    assert any("absent from Cargo.lock" in e for e in errors)


def test_empty_workspace_is_an_error_not_a_pass(tmp_path: Path):
    _manifest(tmp_path, "[workspace]\nmembers = []\n")
    assert check(tmp_path) != []


def test_lockfile_parsing_returns_all_versions(tmp_path: Path):
    _lock(
        tmp_path,
        'version = 4\n\n[[package]]\nname = "a"\nversion = "1.0.0"\n'
        'source = "registry+https://x"\nchecksum = "a"\n\n'
        '[[package]]\nname = "a"\nversion = "2.0.0"\n'
        'source = "registry+https://x"\nchecksum = "b"\n',
    )
    assert load_lockfile(tmp_path) == {"a": ["1.0.0", "2.0.0"]}


# -- the reproducible build itself ------------------------------------------


def test_reproducible_build_script_exists_and_is_executable():
    assert SCRIPT.exists()
    assert SCRIPT.stat().st_mode & 0o111, "script must be executable"


def test_reproducible_build_script_is_valid_bash():
    proc = subprocess.run(["bash", "-n", str(SCRIPT)], capture_output=True, text=True)
    assert proc.returncode == 0, proc.stderr


def test_reproducible_build_script_builds_twice_and_compares_digests():
    body = SCRIPT.read_text(encoding="utf-8")
    assert body.count("cargo build -p price-oracle") == 1
    assert "build_once a" in body and "build_once b" in body
    assert 'if [ "$HASH_A" != "$HASH_B" ]' in body
    assert "--locked" in body


def test_reproducible_build_script_pins_the_canonical_environment():
    body = SCRIPT.read_text(encoding="utf-8")
    for var in ("SOURCE_DATE_EPOCH", "CARGO_INCREMENTAL=0", "TZ=UTC", "LC_ALL=C"):
        assert var in body, var
    assert "rust-toolchain.toml" in body


def test_makefile_build_uses_the_locked_dependency_graph():
    makefile = (REPO_ROOT / "Makefile").read_text(encoding="utf-8")
    build = makefile.split("\nbuild:", 1)[1].split("\n\n", 1)[0]
    assert "--locked" in build


def test_container_image_pins_the_same_toolchain_as_the_toolchain_file():
    channel = re.search(r'channel = "([^"]+)"', TOOLCHAIN.read_text(encoding="utf-8")).group(1)
    dockerfile = DOCKERFILE.read_text(encoding="utf-8")
    assert f"rust:{channel}-" in dockerfile, dockerfile.splitlines()[0]
    assert "wasm32v1-none" in dockerfile
    assert "TZ=UTC" in dockerfile and "CARGO_INCREMENTAL=0" in dockerfile
