-- Tyu Lean 4 port — library root (PLAN-VERIFY-3 P3.1).
--
-- The generated data layer (op enum, semantics table, target parameters,
-- memory-model interfaces) forms the port's *generated* surface; the
-- concrete step semantics (`Tyu.Step`) and the stack algebra (T-C,
-- `Tyu.Sound`) are the P4 substrate; the conformance executable (Main.lean)
-- implements the abstract transfer over the generated layer.
--
-- Authored semantic files (Mem, the P9 source-fragment embedding Src, and
-- the P12 bundle instances Bundles) consume this generated layer; the
-- remaining authored surfaces (Abs, Services) land in their phases.
import Tyu.IR.Op
import Tyu.IR.Semantics
import Tyu.IR.Target
import Tyu.Mem
import Tyu.Bundles
import Tyu.Src
import Tyu.Step
import Tyu.Services
import Tyu.Sound
-- P10: the automation library (tactics + cycle instantiation). The
-- measurement executables (automation_rate, fill) live in their mains.
import Tyu.Automation.Auto
import Tyu.Automation.Cycle
