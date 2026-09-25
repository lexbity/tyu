# Slice-8 verification corpus — a named-predicate contract (Q6): the callee
# traps its own `needs [ pct-in-range ]` against its ⊤ input, so the
# callee-side contract-pre obligation is open by construction (the callee
# cannot know its callers; Q5/Q12 — external tools or a closed world close
# it). Justified in ci/verify-allowlist.txt.
module Contract;
subtype Percent = i64 range 0..100;

: pct-in-range ( Percent -- Percent bool )
  dup 0 >= [ dup 100 <= ] [ 0 0 == ] if ;

: withdraw ( Percent -- bool )
  needs [ pct-in-range ]
  drop true ;

: main ( -- i64 )
  50 as Percent withdraw drop 0 ;
export { main };
end;