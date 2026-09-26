import Tyu.Gen.Stmt
import Tyu.Gen.Sha256
import Tyu.Gen.Render

/-! The Gen subsystem root (PLAN-VERIFY-3 P5).

`Tyu.Gen.Stmt` is the statement-semantics glue the rendered statements rest
on; `Tyu.Gen.Render` is the renderer that produces `Gen/<Module>.lean` from
`tyu.obl/v2` artifacts; `Tyu.Gen.Sha256` is the port's SHA-256 (the
renderer↔encoder drift lock). The committed golden statements
(`goldens/gen/`) elaborate through `Tyu.Gen.Golden`, which is imported only
by the port gate (`ci/port.sh`), keeping the core library lean. -/