- [ ] Gather current admin/source limit patterns (already partially gathered)
- [x] Update storage/config: add DataKey + getter/setter for max oracle sources (default 50)
- [x] Update ErrorCode: add MaxSourcesReached
- [x] Wire admin functions set_max_sources/get_max_sources into contract API + admin module
- [x] Enforce limit in sources::add_source
- [ ] Add unit tests for limit enforcement (including rejection when at limit)
- [ ] Run cargo test to ensure all existing tests pass

