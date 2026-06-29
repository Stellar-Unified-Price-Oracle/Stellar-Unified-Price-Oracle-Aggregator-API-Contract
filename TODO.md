# TODO

## Asset registry lookup optimization (Vec -> efficient index)

- [ ] Create plan + confirm changes across assets/storage/types/lib
- [ ] Implement asset membership index for O(1) lookup without breaking existing `assets()` listing
- [ ] Add backward-compatible migration logic (existing AssetRegistry Vec -> new index)
- [x] Update gas tracking / add benchmark for 50+ assets lookup (get/try asset registered)

- [x] Document memory/storage tradeoff in docs or comments

- [ ] Ensure all existing tests still compile (at least logically) and add new unit test for migration


