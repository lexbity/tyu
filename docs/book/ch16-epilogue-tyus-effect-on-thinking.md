# Chapter 14 — Epilogue: Tyu's Effect on Thinking

> The Vigilance Trap is closed.
> Not because everyone stopped being careful —
> because being careful stopped being the mechanism.
>
> — workshop wall

Chapter 1 opened with two ways to be careful, and promised the second.
Look back at what "moving the promise into the program" turned out to
mean, claim by claim:

| The claim                                          | The check                  | Where it was earned |
|----------------------------------------------------|----------------------------|----------------------|
| "my stack ends where I say"                        | the stack-effect checker   | ch. 2 |
| "my values stay in their ranges"                   | subtypes, trap 21          | ch. 3 |
| "this is true whenever the word runs"              | contracts, trap 20         | ch. 4 |
| "I only do what I declared"                        | the effect matrix          | ch. 5 |
| "my stack never rises past its shape"              | the `(net, high)` monoid   | ch. 6 |
| "one writer, or many readers; custody moves"       | the borrow ledger          | ch. 7 |
| "the hardware is what the descriptor says"         | MMIO checking              | ch. 8 |
| "this word is one sentence"                        | factoring, made mechanical | ch. 9 |
| "the interface is the whole truth"                 | the `.def` mirror          | ch. 10 |
| "the handler and mainline cannot race"             | the cross-context rule     | ch. 11 |
| "the scheduler holds what I handed it"             | effects + annotations      | ch. 12 |
| "this module may run here"                         | the loader's gates         | ch. 13 |
| "this contract is a theorem"                       | the proof pipeline         | ch. 14 |

Twelve claims. None of them is a comment. The daring they licensed — an
interrupt sharing a device, signed code arriving from untrusted media —
was not made safe by a framework or a review process. It was made safe
because each claim got small enough to check, and then the checks got
composed.

Three habits are the book, compressed:

**Say the claim where the machine reads it.** The range in a subtype, the
promise in a `needs` clause, the effect in `performs`, the device in the
descriptor. If a fact is load-bearing, it is not a comment.

**Factor until the proofs are reusable.** The word you name once is the
word whose shape, contract, and bounds are proven at every call site,
forever, for free.

**Know the edge of every proof.** The data stack is proven; the native
stack is not (yet). The ISR budget is checked; the end-to-end interrupt
demo is the open edge. A guarantee that states its boundary is worth a
dozen that pretend to be total — and chapter 14 showed the boundary
moving: the prover exists, the contracts of chapter 4 are theorems when
the developer (or the machine's candidate proofs) close them, and the
boundary's remaining entries — the native stack, the borrow ledger's
formalization, enum conversions, constant folding — are named in the
shipped TCB document precisely so the next rung is an assignment, not a
wish. The habits this book drilled are the diet the prover already eats.

Forth taught that programs are built from words. Ada taught that promises
can be checked. This book's bet is that the two belong together: a
language small enough to hold in your head, with a checker strict enough
that what you hold is the truth. The rigor was never the brake.

It was the engine.

*— end —*
