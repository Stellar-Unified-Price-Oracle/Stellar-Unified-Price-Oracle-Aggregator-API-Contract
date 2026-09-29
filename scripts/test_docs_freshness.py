"""Demonstrates each docs-freshness failure class (#540)."""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("df", Path(__file__).with_name("docs_freshness.py"))
df = importlib.util.module_from_spec(spec)
spec.loader.exec_module(df)


def _doc(tmp_path, text):
    df.ROOT = tmp_path
    p = tmp_path / "doc.md"
    p.write_text(text)
    return p


def test_dead_local_link_fails_unless_allowlisted(tmp_path):
    p = _doc(tmp_path, "[x](missing.md) [y](https://example.invalid/)")
    assert df.check_links(p, set(), external=False) == ["doc.md: missing file missing.md"]
    assert df.check_links(p, {"missing.md"}, external=False) == []


def test_dead_external_link_fails(tmp_path, monkeypatch):
    monkeypatch.setattr(df, "url_ok", lambda u: False)
    p = _doc(tmp_path, "[y](https://example.invalid/)")
    assert df.check_links(p, set(), external=True)
    assert df.check_links(p, {"https://example.invalid*"}, external=True) == []


def test_non_compiling_snippets_fail(tmp_path):
    p = _doc(tmp_path, "```json\n{bad}\n```\n```python\ndef (:\n```\n```bash\nif then\n```\n")
    assert len(df.check_snippets(p)) == 3
    ok = _doc(tmp_path, "```bash\nstellar deploy --id <CONTRACT_ID>\n```\n```json ignore\n{\n```\n")
    assert df.check_snippets(ok) == []


def test_version_mismatched_examples_detected(tmp_path):
    p = _doc(tmp_path, "<!-- contract-version: 0.0.1 -->\n```rust\nclient.gone_fn(&a);\nclient.lastprice(&a);\n```\n")
    errs = df.check_interface(p, {"lastprice"}, "0.1.0")
    assert any("gone_fn" in e for e in errs) and any("0.0.1" in e for e in errs)
    assert not any("lastprice" in e for e in errs)
