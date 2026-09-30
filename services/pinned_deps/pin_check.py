"""Dependency pinning and reproducible-build preconditions (#501).

A contract that cannot be rebuilt from source cannot be verified, so the two
inputs to a build — the dependency graph and the toolchain — are pinned and
checked rather than assumed:

* **Every requirement is exact.** Each dependency in every workspace manifest
  must use a fully-qualified ``=x.y.z`` requirement. ``"26"``, ``"^1.5"`` and
  ``"*"`` are rejected: they are not pins, and they let a fresh resolution
  silently move the artifact.
* **The lockfile is committed and hashed.** Every registry package in
  ``Cargo.lock`` must carry a ``checksum``, so a build consumes content, not a
  name.
* **No drift between manifests and lockfile.** A direct dependency that is not
  satisfied by the locked version, or a git dependency without an immutable
  ``rev``, is rejected. This is what catches "I bumped the lockfile on my
  machine" — the artifact would no longer be the reviewed one.

See ``docs/reproducible-builds.md``.
"""
from __future__ import annotations

import argparse
import re
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Mapping, Optional, Sequence, Tuple

#: A fully pinned requirement: `=1.2.3`, with an optional pre-release/build
#: suffix. Anything else (bare `1`, `^1.2`, `>=1`, `*`) is floating.
EXACT_REQUIREMENT = re.compile(r"^=\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.\-]+)?$")

#: Manifest sections that declare dependencies.
DEPENDENCY_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")


@dataclass(frozen=True)
class Requirement:
    """One declared dependency of one workspace member."""

    manifest: str
    section: str
    name: str
    spec: Mapping[str, object]

    @property
    def version(self) -> Optional[str]:
        value = self.spec.get("version")
        return str(value) if value is not None else None

    @property
    def is_local(self) -> bool:
        return "path" in self.spec

    @property
    def is_git(self) -> bool:
        return "git" in self.spec


def _iter_manifests(root: Path) -> Iterable[Path]:
    """Every manifest of the deployed workspace.

    Scoped to `[workspace] members` (plus the root manifest) on purpose: the
    reproducibility claim is about the graph that produces the deployed WASM.
    `sdk/rust` is a standalone client library outside the workspace, with its
    own (absent) lockfile, and is not part of that artifact.
    """
    root_manifest = root / "Cargo.toml"
    if not root_manifest.exists():
        return
    data = tomllib.loads(root_manifest.read_text(encoding="utf-8"))
    members = (data.get("workspace", {}) or {}).get("members", []) or []
    for member in members:
        member = str(member)
        if any(ch in member for ch in "*?["):
            base = root / member.split("*", 1)[0].rstrip("/")
            yield from sorted(base.rglob("Cargo.toml"))
        else:
            yield root / member / "Cargo.toml"


def load_requirements(root: Path) -> List[Requirement]:
    """Reads every workspace manifest into `Requirement`s."""
    requirements: List[Requirement] = []
    for manifest in _iter_manifests(root):
        data = tomllib.loads(manifest.read_text(encoding="utf-8"))
        for section in DEPENDENCY_SECTIONS:
            deps = data.get(section, {}) or {}
            if not isinstance(deps, dict):
                continue
            for name, spec in deps.items():
                if isinstance(spec, str):
                    spec = {"version": spec}
                if not isinstance(spec, dict):
                    continue
                requirements.append(
                    Requirement(
                        manifest=str(manifest.relative_to(root)),
                        section=section,
                        name=name,
                        spec=spec,
                    )
                )
    return requirements


def load_lockfile(root: Path) -> Dict[str, List[str]]:
    """Maps package name -> locked versions, from `Cargo.lock`."""
    lock = root / "Cargo.lock"
    if not lock.exists():
        return {}
    data = tomllib.loads(lock.read_text(encoding="utf-8"))
    packages: Dict[str, List[str]] = {}
    for package in data.get("package", []) or []:
        packages.setdefault(str(package["name"]), []).append(str(package["version"]))
    return packages


def _version_tuple(version: str) -> Tuple[int, int, int]:
    core = version.split("-", 1)[0].split("+", 1)[0]
    parts = core.split(".")
    while len(parts) < 3:
        parts.append("0")
    return tuple(int(p) for p in parts[:3])  # type: ignore[return-value]


def check_requirements(requirements: Sequence[Requirement]) -> List[str]:
    """Every requirement must be exact, or a git dependency pinned by `rev`."""
    errors: List[str] = []
    for req in requirements:
        where = f"{req.manifest} [{req.section}] {req.name}"
        if req.is_local:
            # Workspace member: pinned by the lockfile and by the manifest's
            # own `version`, so there is nothing floating to reject.
            continue
        if req.is_git:
            if "rev" not in req.spec:
                errors.append(
                    f"{where}: git dependency must pin an immutable `rev`, not a branch"
                )
            continue
        version = req.version
        if version is None:
            errors.append(f"{where}: no version requirement")
        elif not EXACT_REQUIREMENT.match(version):
            errors.append(
                f"{where}: requirement '{version}' is not an exact pin — use "
                f"'=<major>.<minor>.<patch>' so the build cannot float"
            )
    return errors


def check_lockfile(
    requirements: Sequence[Requirement], root: Path
) -> List[str]:
    """The lockfile must exist, be hashed, and match every direct dependency."""
    errors: List[str] = []
    lock = root / "Cargo.lock"
    if not lock.exists():
        return ["Cargo.lock is missing: the resolved graph must be committed"]

    data = tomllib.loads(lock.read_text(encoding="utf-8"))
    locked: Dict[str, List[str]] = {}
    for package in data.get("package", []) or []:
        locked.setdefault(str(package["name"]), []).append(str(package["version"]))
        source = package.get("source")
        if source and str(source).startswith("registry+") and not package.get("checksum"):
            errors.append(
                f"Cargo.lock: {package['name']} {package['version']} has no checksum — "
                "the build must consume hashed content"
            )

    for req in requirements:
        if req.is_local or req.is_git:
            continue
        version = req.version
        if version is None:
            continue
        want = version.lstrip("=")
        available = locked.get(req.name, [])
        if not available:
            errors.append(
                f"{req.manifest} [{req.section}] {req.name}: required but absent from "
                "Cargo.lock — the lockfile is stale"
            )
        elif want not in available:
            errors.append(
                f"{req.manifest} [{req.section}] {req.name}: pinned {want} but "
                f"Cargo.lock holds {', '.join(available)} — dependency drift"
            )
    return errors


def check(root: Path) -> List[str]:
    """Runs every pinning check; returns the errors (empty == reproducible)."""
    requirements = load_requirements(root)
    if not requirements:
        return [f"no workspace manifests with dependencies found under {root}"]
    return check_requirements(requirements) + check_lockfile(requirements, root)


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Dependency pinning check (#501)")
    parser.add_argument(
        "root", nargs="?", default=".", help="repository root (default: cwd)"
    )
    args = parser.parse_args(argv)
    errors = check(Path(args.root))
    for error in errors:
        print(f"::error::{error}")
    print(
        f"{len(load_requirements(Path(args.root)))} requirement(s) checked, "
        f"{len(errors)} error(s)"
    )
    return 1 if errors else 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
