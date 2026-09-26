-- Tyu Lean 4 port — library root (PLAN-VERIFY-3 P3.1).
--
-- The generated data layer (op enum, semantics table, target parameters,
-- memory-model interfaces) forms the port's *generated* surface; the
-- concrete step semantics (`Tyu.Step`) and the stack algebra (T-C,
-- `Tyu.Sound`) are the P4 substrate; the conformance executable (Main.lean)
-- implements the abstract transfer over the generated layer.
--
-- Authored semantic files (Mem/Src/Abs/Services/…) land in later phases,
-- always consuming this generated layer.
import Tyu.IR.Op
import Tyu.IR.Semantics
import Tyu.IR.Target
import Tyu.Mem
import Tyu.Step
import Tyu.Sound
