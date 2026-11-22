<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Timelapse Render Daemon contributors -->

# Fixture spec for check-test-names.py

## 1. Tickets

### TRD-01 — Fixture ticket

## Goal
Name one test of each kind.

## Acceptance criteria
- A1: the first behaviour — check: test `criteria_rust_absent`.
- A2: the second behaviour — check: `CriteriaClassTests.some_member`.
- A3: the third behaviour — check: `tests/criteria_file.rs`.

## Tests
None beyond the criteria.

---

## 2. Test plan

| Area | Automated tests | Kind | Ticket | Runs in |
|---|---|---|---|---|
| fixture area | tests `table_rust_present`, `TableClassTests`, `tests/table_file.rs` | unit | TRD-01 | same |
