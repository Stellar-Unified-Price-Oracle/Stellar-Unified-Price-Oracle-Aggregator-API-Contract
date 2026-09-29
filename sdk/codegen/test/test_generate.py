import json

from sdk.codegen.generate import ABI, drift, gen_python, gen_typescript


def test_committed_bindings_match_abi():
    assert drift(json.loads(ABI.read_text())) == []


def test_drift_is_detected(tmp_path):
    abi = json.loads(ABI.read_text())
    abi["functions"].append({"name": "removed_fn", "inputs": [], "output": "u32"})
    problems = drift(abi)
    assert any("removed_fn" in p for p in problems)
    assert any("out of date" in p for p in problems)


def test_types_map_per_language():
    abi = json.loads(ABI.read_text())
    assert "Promise<PriceData[] | null>" in gen_typescript(abi)
    assert "-> Optional[List[PriceData]]" in gen_python(abi)
