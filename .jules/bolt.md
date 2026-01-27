## 2026-01-25 - [Optimizing Set Intersection in Rust]
**Learning:** `HashSet::intersection` creates a new collection, which causes allocation overhead. For multi-term search queries, iteratively narrowing down results can be optimized by using `retain` on the smaller owned set, avoiding new allocations.
**Action:** When intersecting two owned sets `A` and `B`, check which is smaller. Iterate the smaller one and `retain` elements present in the other.

## 2026-01-25 - [Hoisting Invariant Calculations from Recursion]
**Learning:** In recursive JSON traversal for search, repetitive operations like `to_lowercase()` on the search pattern (which is invariant) cause significant allocation overhead (O(N) allocations where N is total nodes).
**Action:** Hoist invariant transformations (like lowercasing the search pattern) out of the recursive loop. Pass the prepared data down the stack.
