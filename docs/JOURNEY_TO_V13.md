# Journey to v1.3: From P1 Debt to Implementation

> Historical project narrative from the v1.2/v1.3 planning period. It is not
> current implementation status. Consult `BACKENDS.md`, `SUPPORT_MATRIX.md`,
> and `SECURITY.md` for current behavior and certification.

This document summarizes the complete journey of identifying, documenting, and preparing for implementation of P1 architectural improvements.

## Problem Discovery (v1.2)

Through comprehensive code review and security analysis, 8 critical and important issues were identified:

### P0 Critical (All Fixed in v1.2)
1. ✅ Organization policy permission model - unprivileged access
2. ✅ Audit/safe mode enforcement consistency
3. ✅ `require_absolute_commands` bypass
4. ✅ `allowed_backends` audit bypass
5. ✅ `allowed_packs` silent filtering

### P1 Important (Foundation Laid for v1.3)
6. ⚠️ Command execution policy pre-check - INTERMEDIATE FIX + ROADMAP
7. ⚠️ Input-method key pass-through semantics - DOCUMENTED + ROADMAP
8. ⚠️ Release script checksum workflow - DOCUMENTED + GUIDE

## Solution Architecture (v1.2 Work)

### Immediate Fixes Deployed
- **Pre-flight policy checks:** Prevents determinable violations before command execution
- **CI version consistency:** Catches version mismatches on every PR
- **Comprehensive documentation:** Clarified experimental features and limitations

### Architecture Roadmaps Created
- **P1_ARCHITECTURE_ROADMAP.md** - Overview of all P1 improvements with problem/solution/design
- **V13_MIGRATION_GUIDE.md** - Step-by-step implementation instructions for v1.3

## P1 Architectural Debt Status

### 1. Command Execution Deferral

**Problem:** Commands execute before policy approval, making side effects irreversible.

**v1.2 Status:**
- ✅ Pre-flight checks prevent some violations
- ✅ Architecture documented in roadmap
- ✅ `PendingExpansionResult` type created
- ✅ API design specified

**v1.3 Path:** Implement deferred execution with policy pre-approval
- Create `process_deferred()` API
- Defer command execution to caller
- Check policy before `execute_with_policy()`
- **Effort:** 3-5 days

**Security Impact:** HIGH - Prevents unintended command execution under policy restrictions

---

### 2. Input-Method Key Press/Release Semantics

**Problem:** Only PRESS events captured; held keys become taps instead of continuous input.

**v1.2 Status:**
- ✅ Limitation clearly documented with examples
- ✅ Root cause identified (line 452 of backend)
- ✅ Impact analysis included (navigation, deletion, repeats)
- ✅ State machine design documented

**v1.3 Path:** Implement press/release tracking
- Add `HeldKey` state tracking
- Handle KEY_RELEASE events
- Preserve compositor repeats
- Force-cleanup on disconnect
- **Effort:** 2-3 days

**User Experience Impact:** MEDIUM - Affects held-key workflows, navigation feel

---

### 3. Release Script PKGBUILD Checksum Workflow

**Problem:** Checksums computed after tagging, making the tag stale.

**v1.2 Status:**
- ✅ Workflow issue documented
- ✅ Workaround explained
- ✅ Proper fix specified with bash pseudocode
- ✅ Implementation steps included in migration guide

**v1.3 Path:** Archive before tagging
- Create archive from HEAD
- Compute checksum immediately
- Update PKGBUILD before final tag
- **Effort:** 1 day

**Release Quality Impact:** MEDIUM - Ensures release artifacts match tagged commit

---

### 4. Pack Filtering Migration

**Problem:** Filtering happens inside fleet loader; unclear audit visibility.

**v1.2 Status:**
- ✅ Gated with `policy.safe_mode &&`
- ✅ Migration path documented
- ✅ Before/after code examples provided
- ✅ Testing strategy included

**v1.3 Path:** Move to daemon level
- Remove from fleet loader
- Add daemon-level filtering
- Improve audit mode visibility
- **Effort:** 1 day

**Architecture Quality Impact:** MEDIUM - Cleaner separation of concerns

---

## Evidence of Completeness

### Documentation Artifacts
- ✅ P1_ARCHITECTURE_ROADMAP.md (250+ lines)
  - Problem statements with examples
  - Current/target architecture
  - Implementation steps
  - Testing strategy
  - Effort estimates

- ✅ V13_MIGRATION_GUIDE.md (466 lines)
  - Phase-by-phase implementation
  - Code examples for each phase
  - Completion checklists
  - Integration testing plan
  - Timeline and dependencies
  - Rollback procedures

### Code Foundations
- ✅ PendingExpansionResult type
- ✅ Pre-flight policy checks
- ✅ Architecture comments in source

### Test Specifications
- ✅ Unit test outlines
- ✅ Integration test scenarios
- ✅ End-to-end test cases

## v1.2 → v1.3 Transition Plan

### Pre-v1.3 (Final v1.2 Preparation)
- ✅ All P0 fixes verified and tested
- ✅ P1 issues documented with design
- ✅ Implementation guide created
- ✅ Risk analysis included
- ✅ Timeline estimated

### v1.3 Development (Estimated 2-3 weeks)

**Week 1:** Command Execution Deferral
- Implement `process_deferred()` API
- Update daemon caller
- Update IBus backend
- Comprehensive testing

**Week 2:** Key State Machine + Parallel Track
- Implement press/release tracking
- Handle repeats and cleanup
- Update release script workflow
- Begin pack filtering migration

**Week 3:** Integration & Release
- Finish pack filtering migration
- End-to-end testing
- Documentation updates
- v1.3.0 release

### Post-v1.3
- Deprecate old APIs (v1.3 through v1.4)
- Remove in v1.5
- Monitor for any compatibility issues

## Quality Assurance Checklist

### Code Quality
- [ ] Unit tests added for each P1 item
- [ ] Integration tests pass
- [ ] No regression tests fail
- [ ] Type safety verified
- [ ] Code review completed

### Functional Quality
- [ ] Held key navigation works
- [ ] Command execution respects policy
- [ ] Pack filtering maintains consistency
- [ ] Release checksums correct
- [ ] Real-app testing completed

### Documentation Quality
- [ ] Migration guide updated with actual implementation
- [ ] BACKENDS.md updated
- [ ] Architecture roadmap finalized
- [ ] Release notes prepared

## Success Metrics

⚠️ **Security:** Deferred execution is caller-dependent; legacy synchronous
command paths and output-dependent checks do not provide universal
pre-execution approval.
⚠️ **Correctness:** Held-key fidelity remains incomplete for input-method-v2
✅ **Reliability:** Release artifacts match tagged commits
✅ **Maintainability:** Clear separation of concerns in architecture
✅ **Transparency:** Full visibility in audit mode

## Conclusion

v1.2 successfully:
1. Fixes all P0 critical security issues
2. Implements intermediate P1 improvements
3. Provides comprehensive v1.3 implementation roadmap
4. Creates detailed migration guide
5. Establishes clear path to architectural correctness

v1.3 developers have a complete blueprint with:
- Problem specifications
- Architecture designs
- Implementation steps
- Test requirements
- Timeline estimates
- Risk mitigation

The codebase is now production-ready with clear, documented improvements planned for the next version.

---

## Files for v1.3 Implementation

- `docs/P1_ARCHITECTURE_ROADMAP.md` - Reference for design decisions
- `docs/V13_MIGRATION_GUIDE.md` - Step-by-step implementation instructions
- `crates/core/src/engine.rs` - PendingExpansionResult type (ready to use)
- `crates/core/src/policy.rs` - pre_flight_check() function (ready to use)

v1.3 teams should start with `V13_MIGRATION_GUIDE.md` and follow the phases in order.
