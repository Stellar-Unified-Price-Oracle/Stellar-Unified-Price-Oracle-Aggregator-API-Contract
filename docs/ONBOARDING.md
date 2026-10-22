# Contributor Onboarding

A guided path from "interested" to "first merged PR" (#542).

## 1. Pick an area by interest

The map in [`onboarding/map.json`](onboarding/map.json) ties each code area to its
issue track, labels, paths and mentor. The current open issues per area are in
[`onboarding/live.md`](onboarding/live.md) (regenerated weekly), or browse
[open issues](https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/issues)
and [good first issues](https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/labels/good%20first%20issue).

## 2. Get a recommendation

```bash
python3 scripts/onboarding.py recommend --interest rust --level beginner
```

This queries GitHub live and returns open, unassigned issues in areas matching
your interest (`rust`, `python`, `docs`, `security`, `ci`, …) at or below your
level, easiest first. Areas are chosen by interest only, so non-docs
contributors aren't funnelled into documentation.

## 3. Pair with a mentor

1. Comment on the issue: "I'd like to take this — mentor: `<mentor from the recommendation>`".
2. The mentor replies within `mentor_response_days` (3) business days.
3. **Fallback:** no reply in time, or the area has no named mentor → the
   maintainers team (`fallback_mentor`) picks it up; mention them on the issue.

Want to mentor? Add your handle to an area's `mentors` in a PR (CODEOWNERS
routes it to maintainers).

## 4. Set up and ship

`make dev` (see [dev-environment.md](dev-environment.md)), then follow
[CONTRIBUTING.md](../CONTRIBUTING.md).

## Tracking outcomes

Mentors add a row to [`onboarding/outcomes.csv`](onboarding/outcomes.csv) when
pairing and fill `first_pr_merged` on merge. `python3 scripts/onboarding.py outcomes`
reports started/merged/conversion and how often the fallback was needed; review
it quarterly and adjust the map.

## Keeping it current

CI (`onboarding` job) runs `scripts/onboarding.py check` and
`scripts/test_onboarding.py`, failing if a mapped path disappears or the
fallback is missing. The weekly `onboarding-refresh` workflow regenerates
`live.md` from open issues and opens a PR when it changes.
