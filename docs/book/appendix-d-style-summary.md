# Appendix D — Style Summary (one page)

> The book in one page. Each rule has a chapter; each rule is checked by
> something.

## Claims

1. Every word declares its stack effect — and the declaration is true
   (ch. 2).
2. Ranges live in subtypes; choices live in enums; groupings live in
   structs (ch. 3).
3. Contracts are arithmetic about the values in view: no tautologies, no
   effects, no drift (ch. 4). Write them the way the prover will want
   them.
4. Declare what a word does (`performs`) and what it needs granted
   (`requires`); the caller decides (ch. 5).
5. Beyond two or three stack items, name them (`=>`) (ch. 2, 9).
6. Recursion is net-zero per level or it is rejected — keep it that way
   (ch. 6).
7. One writer or many readers, never both; a borrow's life is its
   bracket; custody moves and is never cloned (ch. 7).
8. Hardware is named, never addressed; the descriptor wins every
   disagreement (ch. 8).

## Design

9. Factor until every word passes the one-sentence test; stop before
   shims (ch. 9).
10. Factor along the grammar: predicates keep their subject, loop bodies
    are net-zero, locked sections stay inline (ch. 5, 9).
11. A module's `.def` is its constitution: exports are decisions, the
    mirror never drifts (ch. 10).
12. A shared resource is a declared resource, and the lock is bought only
    where reachability demands it (ch. 11).
13. A worker's quotation carries its annotation; a join lives under its
    discharge (ch. 12).

## Field

14. If a fact is load-bearing, it is not a comment (ch. 1).
15. Checked failures are the cheapest failures: trap codes are verdicts,
    not embarrassments (ch. 1).
16. A proof states its boundary. Say what is proven, what is not, and
    which workstream closes it (ch. 6, 11, 13).
17. The mirror, the wall, and the seal are one mechanism: promises live
    in interfaces, interfaces are hashed, and hashes are refused on
    mismatch (ch. 10, 13).

## The two sentences to carry

- *Everything claimed, the machine checks; claims are cheap, and lies
  are loud.*
- *Safety is not the brake — it is the engine: the rigor licenses the
  daring parts.*
