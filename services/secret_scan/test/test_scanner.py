"""Tests for secret scanning, staging and history (#502)."""
from __future__ import annotations

import json
import subprocess
from datetime import date, timedelta
from pathlib import Path

import pytest

from services.secret_scan.scanner import (
    filter_findings,
    main,
    render_report,
    scan_history,
    scan_staged,
    scan_text,
    scan_tree,
)
from services.static_analysis.policy import AllowlistEntry, load_allowlist

REPO_ROOT = Path(__file__).resolve().parents[3]
SHIPPED_ALLOWLIST = REPO_ROOT / "config" / "secret-scan-allowlist.json"
TODAY = date(2026, 1, 1)

# Assembled from fragments so this test file is not itself a secret-scanner
# finding: the corpus lives in code, not as a literal credential in the tree.
FAKE_AWS_KEY = "AK" + "IA" + "QWER7TYUIOPASDFG"
FAKE_GITHUB_TOKEN = "ghp" + "_" + "a" * 36
FAKE_SOROBAN_SEED = "S" + "A" * 55
PEM = "-----BEGIN " + "RSA PRIVATE KEY-----\nMIIEow==\n" + "-----END RSA PRIVATE KEY-----"


def _ids(text: str):
    return sorted({f.id for f in scan_text(text, "x.rs")})


# -- detection ---------------------------------------------------------------


def test_placeholder_aws_sample_key_is_not_reported():
    # The canonical AWS documentation key contains "EXAMPLE" and is a
    # placeholder, not a leak.
    assert scan_text('aws_key = "AKIAIOSFODNN7' + 'EXAMPLE"', "x.rs") == []


def test_aws_access_key_is_detected():
    assert _ids(f'aws_key = "{FAKE_AWS_KEY}"') == ["SECRET-AWS-ACCESS-KEY-ID"]


def test_github_token_is_detected():
    assert _ids(f'token: {FAKE_GITHUB_TOKEN}') == ["SECRET-GITHUB-TOKEN"]


def test_fine_grained_pat_is_detected():
    pat = "github_pat_" + "b" * 60
    assert _ids(pat) == ["SECRET-GITHUB-FINE-GRAINED-PAT"]


def test_soroban_secret_seed_is_detected():
    assert _ids(f'deploy_key = "{FAKE_SOROBAN_SEED}"') == ["SECRET-SOROBAN-SECRET-SEED"]


def test_private_key_block_is_detected():
    assert _ids(PEM) == ["SECRET-PRIVATE-KEY-BLOCK"]


def test_generic_credential_assignment_is_detected():
    assert "SECRET-GENERIC-ASSIGNMENT" in _ids('api_key = "s3cr3t-' + 'value-1234"')


def test_findings_are_redacted_so_the_report_cannot_leak_the_secret():
    findings = scan_text(f'aws_key = "{FAKE_AWS_KEY}"', "x.rs")
    assert findings[0].redacted not in FAKE_AWS_KEY
    assert FAKE_AWS_KEY not in json.dumps(findings[0].to_dict())


# -- false positives ---------------------------------------------------------


@pytest.mark.parametrize(
    "line",
    [
        'api_key = "example-key-do-not-use"',
        'password = "changeme-please-123"',
        'token = "${GITHUB_TOKEN}"',
        'secret_key = "<your-secret-here>"',
        'api_key = os.environ["API_KEY"]',
        'aws_key = "AKIAEXAMPLEKEYPLACEHOLDER"',
        'password = "xxxxxxxxxxxxxxxx"',
    ],
)
def test_placeholders_and_env_lookups_are_not_findings(line: str):
    assert scan_text(line, "x.rs") == []


def test_short_values_do_not_trip_the_generic_rule():
    assert scan_text('token = "abc"', "x.rs") == []


# -- coverage ----------------------------------------------------------------


def test_scanning_covers_source_config_and_ci_files(tmp_path: Path):
    (tmp_path / "src").mkdir()
    (tmp_path / ".github" / "workflows").mkdir(parents=True)
    (tmp_path / "config").mkdir()
    (tmp_path / "tests" / "fixtures").mkdir(parents=True)
    secret = f'aws_key = "{FAKE_AWS_KEY}"\n'
    for rel in (
        "src/lib.rs",
        ".github/workflows/ci.yml",
        "config/canary-assets.txt",
        "tests/fixtures/sample.toml",
    ):
        (tmp_path / rel).write_text(secret, encoding="utf-8")
    findings = scan_tree([tmp_path])
    assert {str(Path(f.path).relative_to(tmp_path)) for f in findings} == {
        "src/lib.rs",
        ".github/workflows/ci.yml",
        "config/canary-assets.txt",
        "tests/fixtures/sample.toml",
    }


def test_build_output_and_dependencies_are_skipped(tmp_path: Path):
    (tmp_path / "target").mkdir()
    (tmp_path / "node_modules").mkdir()
    (tmp_path / "target" / "x.rs").write_text(f'k = "{FAKE_AWS_KEY}"\n', encoding="utf-8")
    (tmp_path / "node_modules" / "y.js").write_text(
        f'k = "{FAKE_AWS_KEY}"\n', encoding="utf-8"
    )
    assert scan_tree([tmp_path]) == []


# -- allowlist ---------------------------------------------------------------


def _entry(**kwargs) -> AllowlistEntry:
    base = dict(
        id="SECRET-AWS-ACCESS-KEY-ID",
        kind="secret",
        owner="alice",
        expires=(TODAY + timedelta(days=30)).isoformat(),
        reason="documentation sample",
        paths=("tests/fixtures/*",),
    )
    base.update(kwargs)
    return AllowlistEntry(**base)  # type: ignore[arg-type]


def test_fixture_secret_is_allowlistable():
    findings = scan_text(f'k = "{FAKE_AWS_KEY}"\n', "tests/fixtures/sample.toml")
    new, allowed = filter_findings(findings, [_entry()])
    assert new == [] and len(allowed) == 1


def test_allowlisting_a_fixture_does_not_weaken_scanning_elsewhere():
    findings = scan_text(f'k = "{FAKE_AWS_KEY}"\n', "src/lib.rs")
    new, allowed = filter_findings(findings, [_entry()])
    assert len(new) == 1 and allowed == []


def test_allowlist_entry_needs_an_owner_and_an_expiry():
    assert any("no owner" in e for e in _entry(owner="").errors(TODAY))
    assert any("expired" in e for e in _entry(expires="2020-01-01").errors(TODAY))
    assert _entry().errors(TODAY) == []


def test_blanket_allowlist_is_rejected():
    assert any("blanket path scope" in e for e in _entry(paths=("**",)).errors(TODAY))


def test_shipped_allowlist_is_valid():
    entries, errors = load_allowlist(SHIPPED_ALLOWLIST)
    assert errors == [], errors
    assert entries == []


# -- the repository is clean -------------------------------------------------


def test_repository_tree_has_no_secrets():
    findings = scan_tree([REPO_ROOT])
    assert findings == [], [f.to_dict() for f in findings[:5]]


def test_repository_allowlist_file_is_present_and_valid():
    assert SHIPPED_ALLOWLIST.exists()
    _, errors = load_allowlist(SHIPPED_ALLOWLIST)
    assert errors == []


# -- staged + history --------------------------------------------------------


def _git_repo(tmp_path: Path) -> Path:
    repo = tmp_path / "repo"
    repo.mkdir()
    env = {
        "GIT_AUTHOR_NAME": "t",
        "GIT_AUTHOR_EMAIL": "t@example.invalid",
        "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@example.invalid",
        "PATH": "/usr/bin:/bin:/usr/local/bin",
        "HOME": str(tmp_path),
    }
    subprocess.run(["git", "init", "-q"], cwd=repo, check=True, env=env)
    return repo


def _commit(repo: Path, name: str, body: str) -> None:
    (repo / name).write_text(body, encoding="utf-8")
    env = {
        "GIT_AUTHOR_NAME": "t",
        "GIT_AUTHOR_EMAIL": "t@example.invalid",
        "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@example.invalid",
        "PATH": "/usr/bin:/bin:/usr/local/bin",
        "HOME": str(repo),
    }
    subprocess.run(["git", "add", name], cwd=repo, check=True, env=env)
    subprocess.run(["git", "commit", "-q", "-m", "add"], cwd=repo, check=True, env=env)


def test_staged_scan_catches_a_secret_before_it_is_committed(tmp_path: Path):
    repo = _git_repo(tmp_path)
    _commit(repo, "safe.rs", "pub fn add() {}\n")
    (repo / "leak.rs").write_text(f'k = "{FAKE_AWS_KEY}"\n', encoding="utf-8")
    assert scan_staged(repo) == []  # not staged yet
    subprocess.run(
        ["git", "add", "leak.rs"],
        cwd=repo,
        check=True,
        env={"PATH": "/usr/bin:/bin", "HOME": str(repo)},
    )
    findings = scan_staged(repo)
    assert len(findings) == 1
    assert findings[0].path == "leak.rs"


def test_history_scan_reports_a_secret_that_was_already_removed(tmp_path: Path):
    repo = _git_repo(tmp_path)
    _commit(repo, "leak.toml", f'aws_key = "{FAKE_AWS_KEY}"\n')
    (repo / "leak.toml").unlink()
    env = {"PATH": "/usr/bin:/bin", "HOME": str(repo)}
    subprocess.run(["git", "add", "-A"], cwd=repo, check=True, env=env)
    subprocess.run(["git", "commit", "-q", "-m", "remove"], cwd=repo, check=True, env=env)

    assert scan_tree([repo]) == []  # gone from the working tree
    findings = scan_history(repo)
    assert len(findings) == 1
    assert findings[0].path == "leak.toml"
    assert findings[0].commit  # the commit that introduced it is identified


def test_history_scan_is_clean_on_this_repository():
    findings = scan_history(REPO_ROOT, limit=400)
    assert findings == [], [f.to_dict() for f in findings[:5]]


# -- CLI ---------------------------------------------------------------------


def test_cli_fails_on_a_committed_fake_secret(tmp_path: Path, capsys):
    allowlist = tmp_path / "allowlist.json"
    allowlist.write_text(json.dumps({"entries": []}), encoding="utf-8")
    (tmp_path / "deploy.toml").write_text(f'aws_key = "{FAKE_AWS_KEY}"\n', encoding="utf-8")
    code = main([str(tmp_path), "--allowlist", str(allowlist)])
    assert code == 1
    assert "SECRET-AWS-ACCESS-KEY-ID" in capsys.readouterr().out


def test_cli_passes_and_writes_a_report_for_a_clean_tree(tmp_path: Path, capsys):
    allowlist = tmp_path / "allowlist.json"
    allowlist.write_text(json.dumps({"entries": []}), encoding="utf-8")
    (tmp_path / "ok.toml").write_text('api_key = "${API_KEY}"\n', encoding="utf-8")
    report = tmp_path / "report.md"
    code = main([str(tmp_path), "--allowlist", str(allowlist), "--report", str(report)])
    assert code == 0
    assert report.read_text(encoding="utf-8").startswith("# Secret scan report")
    assert "0 new secret(s)" in capsys.readouterr().out


def test_cli_fails_on_an_expired_allowlist_entry(tmp_path: Path, capsys):
    allowlist = tmp_path / "allowlist.json"
    allowlist.write_text(
        json.dumps({"entries": [_entry(expires="2020-01-01").to_dict()]}), encoding="utf-8"
    )
    (tmp_path / "ok.toml").write_text("nothing here\n", encoding="utf-8")
    assert main([str(tmp_path), "--allowlist", str(allowlist)]) == 1
    assert "expired" in capsys.readouterr().out


def test_report_mentions_every_section():
    findings = scan_text(f'k = "{FAKE_AWS_KEY}"\n', "a.rs")
    report = render_report(findings, [], findings)
    assert "## Working tree" in report
    assert "## Allowlisted" in report
    assert "## History" in report
    assert FAKE_AWS_KEY not in report
