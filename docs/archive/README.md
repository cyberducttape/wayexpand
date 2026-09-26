# Archived Documentation

This directory contains historical documents that are no longer part of the
active documentation set. They may be useful for understanding past design
decisions or release history, but should not be used for current operational
or development guidance.

## Contents

### Completed/Superseded Checklists
- **RELEASE_1.0_CHECKLIST.md** - Milestone checklist for the 1.0 release (completed)
- **PACKAGING_CHECKLIST.md** - Pre-1.0 packaging verification steps (outdated)

### Historical Design & Security Records
- **SECURITY_AUDIT.md** - Self-audit from the 1.0 release period
- **CLIPBOARD_INJECTION.md** - Documents clipboard-based text injection (removed from daemon due to security concerns)
- **DESKTOP_STATUS.md** - Desktop status integration planning
- **ENTERPRISE_ROADMAP.md** - Enterprise adoption roadmap
- **GNOME_WINDOW_TRACKING.md** - GNOME window tracking exploration
- **PHASE3_DAEMON_INTEGRATION.md** - Integration planning
- **WLROOTS_WINDOW_TRACKER_PLAN.md** - wlroots window tracking planning (not yet implemented)
- **WLROOTS_WINDOW_TRACKING_GUIDE.md** - wlroots implementation guide

### v1.3 Development Records
- **2026-09/** - September 2026 audit findings and development tracking
  - V13_CRITICAL_REGRESSIONS.md - v1.3 regression findings
  - V13_COMPLETION_STATUS.md - v1.3 completion checklist
  - V13_MIGRATION_GUIDE.md - v1.3 upgrade guide
  - AUDIT_FINDINGS.md - Comprehensive audit findings
  - And other development records

## Removed Files (Consolidated)

- **wiki/** - Outdated duplicate documentation (removed Sept 26, 2026)
  - Was superseded by expanded active documentation in parent directory
  - Getting-Started.md → GETTING_STARTED.md
  - Troubleshooting.md → TROUBLESHOOTING.md
  - GUI.md → GUI.md (expanded)
  - Configuration.md → Covered in COMPATIBILITY.md
  - Operations.md → OPERATIONS.md (expanded)
  - Security.md → SECURITY.md (root level)
  - Contributing.md → DEVELOPMENT.md

## Current Documentation

For current operational, development, and compatibility information, refer to:
- [`docs/DOCUMENTATION_INDEX.md`](../DOCUMENTATION_INDEX.md) - Complete documentation guide
- [`docs/SUPPORT_MATRIX.md`](../SUPPORT_MATRIX.md) - Current backend support and compatibility
- [`docs/GETTING_STARTED.md`](../GETTING_STARTED.md) - Setup and installation
- [`docs/RELEASING.md`](../RELEASING.md) - Release procedures
- [`SECURITY.md`](../../SECURITY.md) - Security model and guarantees
