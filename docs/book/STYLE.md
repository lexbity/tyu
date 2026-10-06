# Tyu Book — Voice and Style Guide (v4 calibration)

This file is the authority on how the book sounds. The README's old author
grammar ("prefer the imperative and declarative sentences") prescribed the
register it was trying to avoid; those rules are retired. The labs — code,
transcripts, expected verdicts — are fixed. Everything else serves the voice.

## The reader

One reader: a curious beginner with no prior programming experience and the
toolchain installed. The prose talks to that reader the way a knowledgeable
friend would — thinking aloud, working through what's on the screen, saying
"here" and "look" instead of "note that" and "it must be understood that."

## The register

1. **Conversational.** Contractions are allowed and normal. The narrator may
   ask a question and work through it on the page ("So where exactly did it
   die?"), may notice something odd, may find something surprising. This is
   curiosity, not a formula: no template narration ("we expected X and were
   wrong"), no fake wonder. The questions are ones the narrator genuinely
   works through, in front of the reader.
2. **Show, then name.** Phenomenon first — transcript, trace, scenario; then
   a beat in which the reader can see it; then the mechanism's name. The
   reader is allowed to arrive at the point a sentence before the text says
   it.
3. **No announced significance.** Never "this is the whole chapter," "the
   thesis," "hold that thought," "sit with that," "read that again," "the
   lesson has sharpened." If the passage matters, that is visible. Stage
   directions are not experience.
4. **Metaphor discipline.** One sustained metaphor per abstract concept,
   extended across chapters (plates for the stack), and new mechanisms
   stretch it rather than replacing it. A callback must be to something the
   book actually built: the reader who can't place a figure of speech has
   been handed noise, which is worse than no figure at all. (Lesson from
   calibration: "the kitchen already knows it" — the plates chapter built a
   dishwasher, never a kitchen.)
5. **Aphorisms are earned.** At most one per section, and only a conclusion
   the reader could have reached themselves one sentence earlier.
6. **Motif budget.** claim / lie / verdict / truth / promise: roughly two
   uses per chapter each. They were fresh in ch. 1; by ch. 13 they are
   wallpaper. Vary the vocabulary instead of re-tuning the drum.
7. **Imperatives belong in lab instructions** ("run it," "change 50 to 150"),
   where they are natural. The prose between labs does not command.

## Ban list (absolute)

- The "X is not Y — it is Z" corrective, and its siblings ("not X, just Y";
  "X isn't a N, it's a M"). Banned outright, in every section, including
  epigraphs and closers.
- Bolding a sentence to make it a thesis.
- "in miniature," "the whole chapter," "the whole game," "the whole
  personality," and comparable emphasis inflation.
- Self-praise of the material ("the most beautiful check").
- Stage directions standing in for experience.
- Assorted tics carried over from the v3 draft: "as just seen," "worth
  memorizing," "deserves a slow read."

## Audience: the "In more depth" section

The baseline is the beginner. Every chapter carries one clearly-marked
subsection, titled **In more depth**, in a fixed position (after the labs,
before the exercises). It is where material that needs prior experience
lives: C/embedded war stories and anti-patterns, comparisons to Rust, Ada,
seL4, s2n-bignum, toolchain internals and verification policy, registry-level
detail. Test for placement: if a sentence needs prior C (or Rust, or the
repo's CI layout) to land, it belongs there — no exceptions, including
chapter openings. A reader skipping every "In more depth" section must never
lose the thread.

The v3 "Post-mortem (optional — for readers with C or embedded scars)" and
"From the workbench" blocks dissolve into this section or into the main
prose, as the material dictates.

## Chapter skeleton

1. **Open** — a real moment: a transcript, a trace, a scenario. Not an
   epigraph plus a thesis statement.
2. **Development** — conversational sections building the discipline;
   show-then-name; the chapter's metaphor carried through, not dropped after
   the introduction.
3. **Labs** — green / red / engine, full source inline, real transcripts
   (unchanged from v3; the code is the fixed point).
4. **In more depth** — the experienced-engineer subsection (see above).
5. **Exercises** — predict-then-run preferred over filler.

The v3 "**The Practice**" block dissolves: its sentence becomes ordinary
prose in the chapter's opening section — no blockquote, no bold label.

## Integrity (unchanged and non-negotiable)

- Transcripts are real. Never call the text or the language "honest" — that
  is for the reader to judge.
- Edges and unproven halves are shown plainly.
- Cross-references, section numbers, counts, and chapter numbers are checked
  facts. A wrong "§3.4" or a "three species" sentence over a nine-row table
  breaks the same contract as a fabricated transcript.

## Calibration examples

These are the target register, approved in calibration. Use them as
reference when re-narrating.

### A — ch. 1, after the engine trap

Before (v3):

> Read that pair of facts, because they are the whole chapter. … Exit code 20
> is not a crash. It is a *verdict* … **Checked failures are the cheapest
> failures a program can have.**

After:

> The transcript is worth a second look, because something odd happened in
> it. `level accepted` printed once — for the 42. Then `main` called
> `set-level` with -5, and after that… nothing. No error message, no second
> line, no `S`. The process is just gone, and its exit status is 20.
>
> So where exactly did it die? `main` still had two lines left — the log and
> the exit — and neither ran. In fact, `set-level`'s body never ran either.
> The only thing that *did* run was the check: `needs` placed it at the top
> of the word, it asked its one question — is −5 at least zero? — got its
> answer, and ended the process right there. The buggy call never happened,
> as far as the program's own code is concerned.
>
> The exit status is how the runtime explains that. 20 is `CONTRACT_FAIL`,
> one of a small set of named verdicts:
>
> [trap table]
>
> A crash in most languages tells you where the memory gave up. A trap like
> this tells you which promise broke — and it caught the call while the
> damage was still one word wide, before anything downstream ever saw the
> negative level.

### B — ch. 6, net and high

Before (v3):

> The second line deserves a slow read: B's peak is *shifted up* by whatever
> A left behind… It is associative… which makes the shapes a *monoid*.

After:

> Here's the rule for phrases that run one after the other. Whatever A
> leaves on the table is what B starts with. The net of A-then-B is the two
> nets added. The peak is whichever is worse: A's own peak, or A's leftovers
> plus B's peak. `dup dup` on its own never gets past two plates; run it
> right after a `dup` and it touches three — same phrase, taller move,
> because the table was already occupied.
>
> The rule doesn't care how you bracket things, either. (A;B);C lands in the
> same place as A;(B;C), the way (2+3)+4 and 2+(3+4) both land on 9. So the
> checker can walk a word body once, left to right, keeping a running
> (net, high) on a scrap of paper, and never go back. Mathematicians call a
> combine-rule that ignores grouping and has a do-nothing element a monoid;
> the name matters less than what it buys — one pass, no revisiting, no
> doubt.
