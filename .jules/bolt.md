## 2026-01-25 - [Optimizing Set Intersection in Rust]
**Learning:** `HashSet::intersection` creates a new collection, which causes allocation overhead. For multi-term search queries, iteratively narrowing down results can be optimized by using `retain` on the smaller owned set, avoiding new allocations.
**Action:** When intersecting two owned sets `A` and `B`, check which is smaller. Iterate the smaller one and `retain` elements present in the other.
