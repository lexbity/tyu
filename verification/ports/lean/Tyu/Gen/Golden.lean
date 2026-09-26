import Tyu.Gen.Golden.Bank
import Tyu.Gen.Golden.Clean
import Tyu.Gen.Golden.Contract
import Tyu.Gen.Golden.EventLoop
import Tyu.Gen.Golden.Lending
import Tyu.Gen.Golden.LoopSub
import Tyu.Gen.Golden.OpenCast
import Tyu.Gen.Golden.Post

/-! The statement goldens (P5): the committed generated statements for the
corpus, imported so `lake build` elaborates them (the elaboration gate). The
`gen` renderer's output is byte-compared against these files
(`ci/port.sh`); their statement hashes are re-checked against the Rust
encoder's by `crates/tooling-tests/tests/gen_statement_drift.rs`. -/
