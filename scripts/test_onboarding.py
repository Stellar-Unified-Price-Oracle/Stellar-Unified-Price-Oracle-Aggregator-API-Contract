"""Onboarding map tests (#542): map is current, recommendations, fallback, outcomes."""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("ob", Path(__file__).with_name("onboarding.py"))
ob = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ob)


def issue(n, track, diff, **kw):
    return {"number": n, "title": f"t{n}", "html_url": f"u{n}", "state": "open", "labels": [],
            "assignees": [], "body": f"> **Difficulty:** x {diff} · **Track:** {track}\n", **kw}


def test_committed_map_is_current():
    # Fails when a mapped path is moved/deleted, so the map is kept up to date by CI.
    assert ob.check_map(ob.load_map()) == []


def test_stale_path_and_missing_fallback_detected(tmp_path):
    m = {"areas": [{"id": "a", "title": "A", "track": "X", "interests": ["rust"], "paths": ["nope"]}]}
    errs = ob.check_map(m, tmp_path)
    assert "no fallback_mentor defined" in errs and any("nope" in e for e in errs)


def test_recommend_filters_by_interest_level_and_state():
    m = ob.load_map()
    issues = [
        issue(1, "Core Protocol", "Advanced"),
        issue(2, "Core Protocol", "Intermediate"),
        issue(3, "Core Protocol", "Beginner", assignees=[{"login": "x"}]),
        issue(4, "Developer Experience", "Beginner"),
        issue(5, "Core Protocol", "Beginner", pull_request={}),
    ]
    got = [r["number"] for r in ob.recommend(issues, m, "rust", "intermediate")]
    assert got == [2]
    assert [r["number"] for r in ob.recommend(issues, m, "docs", "beginner")] == [4]
    assert [r["number"] for r in ob.recommend(issues, m, None, "advanced")] == [4, 2, 1]


def test_unpaired_area_falls_back_to_maintainers():
    m = ob.load_map()
    rec = ob.recommend([issue(9, "Testing / Verification", "Intermediate")], m, None, "advanced")
    assert rec[0]["mentor"] == m["fallback_mentor"]


def test_outcomes_summary(tmp_path):
    p = tmp_path / "o.csv"
    p.write_text("contributor,area,mentor,first_issue,started,first_pr_merged\n"
                 "a,core,@m,1,2026-09-01,2026-09-10\nb,devx,@org/maintainers,2,2026-09-02,\n")
    assert ob.summarize_outcomes(p) == {"started": 2, "merged": 1, "conversion": 0.5, "fallback_pairings": 1}


def test_live_render_links_issues():
    m = ob.load_map()
    out = ob.render_live([issue(7, "Core Protocol", "Advanced")], m)
    assert "[#7 t7](u7)" in out and "_no open issues right now_" in out
