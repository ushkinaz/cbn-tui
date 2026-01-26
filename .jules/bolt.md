## 2024-05-22 - [Batching Index Updates]
**Learning:** Inserting into a global `HashMap<String, HashSet<usize>>` inside a tight loop for every word in every item causes massive overhead due to repeated hashing and set checks for the same word in the same item.
**Action:** Collect unique values for the current item into a local `HashSet` first, then update the global index in a batch. This reduces N lookups to 1 lookup per unique word.
