import Std

namespace Tyu.IR

/- The port's generated data layer (PLAN-VERIFY-3 P3.1), rendered by
`crates/verifier/src/export/lean.rs` — DO NOT EDIT. Mirrors
`verifier::target::TARGETS`. "Semantics is a function of (TargetSpec,
MemModel-instance)" (§Q3): the four fields relativize every statement. -/
/-- The target-identity record (`verifier::target::TargetSpec`): triple,
data-stack slot width, integer width, ABI arch tag. -/
structure TargetSpec where
triple : String
slotBytes : Nat
wordBits : Nat
archTag : Nat
deriving DecidableEq, Repr, Inhabited

/-- The recognized targets, in `verifier::target::TARGETS` order. -/
def x86_64_unknown_linux_gnu : TargetSpec := { triple := "x86_64-unknown-linux-gnu", slotBytes := 8, wordBits := 64, archTag := 1 }
def x86_64_unknown_none : TargetSpec := { triple := "x86_64-unknown-none", slotBytes := 8, wordBits := 64, archTag := 1 }
def armv7m_unknown_none : TargetSpec := { triple := "armv7m-unknown-none", slotBytes := 4, wordBits := 32, archTag := 2 }
def riscv32_unknown_none : TargetSpec := { triple := "riscv32-unknown-none", slotBytes := 4, wordBits := 32, archTag := 3 }

def TARGETS : List TargetSpec := [ x86_64_unknown_linux_gnu,  x86_64_unknown_none,  armv7m_unknown_none,  riscv32_unknown_none ]

theorem target_count : TARGETS.length = 4 := by
  decide

end Tyu.IR
