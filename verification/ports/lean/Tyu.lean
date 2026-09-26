-- Tyu Lean 4 port — library root (PLAN-VERIFY-3 P3.1).
--
-- The generated data layer (op enum, semantics table, target parameters,
-- memory-model interfaces) forms the port's part of the *generated*
-- surface; the conformance executable (Main.lean) implements the abstract
-- transfer over it. Authored files (Step/Mem/Src/Abs/…) land in later
-- phases (P4+), always consuming this generated layer.
import Tyu.IR.Op
import Tyu.IR.Semantics
import Tyu.IR.Target
import Tyu.Mem
