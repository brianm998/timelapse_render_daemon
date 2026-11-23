# 1013: XMP mode ties pick the smallest key, as the Swift code does

## Decision

`xmp::assignments` breaks a count tie in favour of the lexicographically smallest value. For
`Model` with `EOS R5` and `EOS R6` each seen once, the embedded model is `EOS R5`.

The TRD-10 spec's Approach text says "descending-key ties". The Swift `XMPFieldMapper.mode`
doc comment says the same. The Swift code does not do that.

## Why

Swift's `mode` calls `max(by:)` with the comparator `a.key > b.key` on ties. `max(by:)` returns the
element that is greatest under that ordering, and that is the element with the smallest key. The
TRD-10 acceptance criterion A1 requires the output to equal the Swift output, so the port follows
the code.

## How to apply

If the owner wants the descending-key rule, change the tie-break in `xmp::mode` and the expected
value in `assignments_match_swift` and `mode_ties_pick_smallest_key` together. Those are the only
tests that pin the rule.
