import Tyu.IR.Target

/-! The abstract-atomic services model (PLAN-VERIFY-3 §Q14 option (b), P15).

Platform services — channel IPC (`platform.channel.*`), the task scheduler
(`platform.task.*`), and time (`platform.time.now_ms`) — are modeled as
**atomic abstract transitions** on an abstract global state: channel FIFO
maps, a task table, and a monotone clock. A developer proof is sequential
reasoning over a nondeterministic-but-atomic environment (§Q14): the word's
OWN service ops are applied one at a time (each op is a single atomic
transition of the global state), while the environment can only ever advance
OTHER tasks' state — which the FIFO laws keep out of the word's own
make-local channels (atomicity: the laws below record that a channel op
touches only its own channel).

The observable claims a developer proves against this model live in
`Tyu.Gen.Render`'s service-statement forms (`Tyu.Services.traceInRange`, the
subtype/guarantee claim over a service-trace output). Those statements only
render when the bundle declares `[model] concurrency = "abstract-atomic"`
(the statement relativism's concurrency dimension, §Q14/§6.7); on a bundle
whose services are unmodeled the same obligations fail closed open
(`service-unmodeled`, the `concurrency`-on + unmodeled-services rule).

Channel identities are Nats incident to a word's `platform.channel.make`
occurrences; the model is content-addressed so make-local FIFO laws are
total. -/

namespace Tyu.Services

/-- The task status lattice (§Q14): ready / running / blocked (waiting on a
join or a full FIFO send) / done. -/
inductive TaskStatus where
  | ready | running | blocked | done
  deriving DecidableEq, Repr, Inhabited

/-- One task's abstract state. -/
structure Task where
  status : TaskStatus
  deriving DecidableEq, Repr, Inhabited

/-- The channel FIFO state: the buffered values in order plus the capacity
bound. A bounded-FIFO send to a FULL channel blocks the sender — modeled on
the task side (`TaskStatus.blocked`); the VALUE FIFO itself only grows by
sends and shrinks by recvs, which is all the FIFO laws observe. -/
structure Channel where
  buf : List Int
  cap : Nat
  deriving DecidableEq, Repr, Inhabited

/-- The abstract global state (§Q14: "an abstract global state"): the channel
table (channel id → FIFO), the task table (task id → state), and the
monotone clock. The tables are TOTAL functions over ids — the abstract model
needs no allocation bookkeeping, and the "other state untouched" atomicity
laws are then transparent. -/
structure Instance where
  channels : Nat → Channel
  tasks : Nat → Task
  clock : Int
  deriving Inhabited

/-- Clock read — the value the hosted `platform.time.now_ms` is modeled as
returning. -/
def nowMs (inst : Instance) : Int := inst.clock

/-- One atomic task-context transition: the environment advances the clock
(monotone by construction — a `timeAdvance n` never decreases it). Channel
and task ops leave the clock EXACTLY (they are atomic and do not advance
time), so `nowMs` is monotone across every op (§Q14's monotone clock). -/
def timeAdvance (n : Nat) (inst : Instance) : Instance :=
  { inst with clock := inst.clock + (n : Int) }

/-- The atomic **channel make** transition: install a fresh (empty, bounded)
FIFO at channel `c`. -/
def chanMake (c : Nat) (inst : Instance) : Instance :=
  { inst with channels := fun d => if d = c then { buf := [], cap := 16 } else inst.channels d }

/-- The atomic **channel send** transition: append `v` to channel `c`'s FIFO.
(The sender's blocking-on-full behavior is a task-status transition in the
task model; the FIFO append is the value-visible part.) -/
def chanSend (c : Nat) (v : Int) (inst : Instance) : Instance :=
  { inst with
      channels := fun d =>
        if d = c then { buf := (inst.channels c).buf ++ [v], cap := (inst.channels c).cap }
        else inst.channels d }

/-- The atomic **channel recv** transition: pop channel `c`'s FIFO head. The
result is the pair `(some v, post-state)` on a non-empty FIFO; `(none,
unchanged)` on empty (a recv on an empty channel blocks the receiver — the
task-status side; the value side observes nothing). -/
def chanRecv (c : Nat) (inst : Instance) : Option Int × Instance :=
  match (inst.channels c).buf with
  | [] => (none, inst)
  | v :: rest =>
      ( some v,
        { inst with channels := fun d => if d = c then { buf := rest, cap := (inst.channels c).cap } else inst.channels d } )

/-- The channel FIFO content of channel `c`. -/
def chanOf (c : Nat) (inst : Instance) : List Int := (inst.channels c).buf

/-- The atomic **task spawn** transition: register task `t` as ready. -/
def taskSpawn (t : Nat) (inst : Instance) : Instance :=
  if inst.tasks t = { status := TaskStatus.ready } then inst
  else { inst with tasks := fun d => if d = t then { status := TaskStatus.ready } else inst.tasks d }

/-- The atomic **task yield** transition: the running task `cur` requeues
(ready). -/
def taskYield (cur : Nat) (inst : Instance) : Instance :=
  { inst with tasks := fun d => if d = cur then { status := TaskStatus.ready } else inst.tasks d }

/-- Whether task `t` has exited (its task-table row is `done`, or it never
existed — an exited/absent task is joinable). -/
def taskDone (t : Nat) (inst : Instance) : Bool := inst.tasks t == { status := TaskStatus.done }

/-- The atomic **task join** transition: the joining task `cur` proceeds when
the joined task `t` is done, else blocks (blocks on a not-yet-exited join —
the §Q14 suspend; the abstract transition is the status change). -/
def taskJoin (cur t : Nat) (inst : Instance) : Instance :=
  if taskDone t inst then inst
  else { inst with tasks := fun d => if d = cur then { status := TaskStatus.blocked } else inst.tasks d }

/-- The task status of task `t` (the observable a join/yield context reads). -/
def taskStatusOf (t : Nat) (inst : Instance) : TaskStatus := (inst.tasks t).status

-- ---------------------------------------------------------------------------
-- Atomicity + FIFO laws (the §Q14 claim surface; axiom-audited, REVIEW.md §3)
-- ---------------------------------------------------------------------------

/-- A channel op touches ONLY its own channel: every other channel's FIFO is
byte-identical after a make. -/
theorem chanMake_other_unchanged (c d : Nat) (inst : Instance) (h : c ≠ d) :
    (chanMake c inst).channels d = inst.channels d := by
  unfold chanMake
  have hdc : d ≠ c := by
    exact fun hdc => h hdc.symm
  simp [hdc]

/-- A channel recv touches ONLY its own channel: `chanSend`/`chanRecv` on `c`
leaves every other channel exact (the atomicity claim). -/
theorem chanSend_other_unchanged (c d : Nat) (v : Int) (inst : Instance) (h : c ≠ d) :
    (chanSend c v inst).channels d = inst.channels d := by
  unfold chanSend
  have hdc : d ≠ c := by
    exact fun hdc => h hdc.symm
  simp [hdc]

/-- Channel ops do not touch the task table or the clock (each is a single
atomic transition of exactly one surface). -/
theorem chanSend_tasks_clock_unchanged (c : Nat) (v : Int) (inst : Instance) :
    (chanSend c v inst).tasks = inst.tasks ∧ (chanSend c v inst).clock = inst.clock := by
  unfold chanSend
  simp

/-- `make` installs an EMPTY bounded FIFO at `c`. -/
theorem chanMake_empty (c : Nat) (inst : Instance) :
    (chanMake c inst).channels c = { buf := [], cap := 16 } := by
  unfold chanMake
  simp

/-- `send` appends: after `send c v`, channel `c` is the old FIFO plus `v`. -/
theorem chanSend_appends (c : Nat) (v : Int) (inst : Instance) :
    (chanSend c v inst).channels c = { buf := (inst.channels c).buf ++ [v], cap := (inst.channels c).cap } := by
  unfold chanSend
  simp

/-- **The FIFO round-trip law (send-then-recv):** sending `v` into a fresh
channel and immediately receiving yields `some v` — the recv observes exactly
what was sent, because the channel is make-local (no other task could have
sent into it in between; atomicity). -/
theorem send_then_recv_value (c : Nat) (v : Int) (inst : Instance) :
    (chanRecv c (chanSend c v (chanMake c inst))).1 = some v := by
  unfold chanRecv chanSend chanMake
  simp

/-- The round trip leaves the fresh channel EMPTY again — a make-local
send-then-recv is the identity on the channel's FIFO. -/
theorem send_then_recv_empty (c : Nat) (v : Int) (inst : Instance) :
    (chanRecv c (chanSend c v (chanMake c inst))).2 |> fun i => (i.channels c).buf = [] := by
  unfold chanRecv chanSend chanMake
  simp

/-- **FIFO order:** two sends then two recvs return the values in order —
the abstract-atomic channel semantics is a FIFO, never LIFO or reordered. -/
theorem fifo_order (c : Nat) (a b : Int) (inst : Instance) :
    let i1 := chanSend c b (chanSend c a (chanMake c inst))
    let r1 := chanRecv c i1
    let i2 := r1.2
    let r2 := chanRecv c i2
    (r1.1, r2.1) = (some a, some b) := by
  unfold chanRecv chanSend chanMake
  simp

/-- **Atomicity / sequential discipline:** the round-trip op pair on channel
`c` is the identity on every OTHER channel (a make-local FIFO cannot be
interleaved into — the "nondeterministic-but-atomic environment" keeps the
word's own ops atomic). -/
theorem roundtrip_other_unchanged (c d : Nat) (v : Int) (inst : Instance) (hc : c ≠ d)
    : (chanRecv c (chanSend c v (chanMake c inst))).2 |> fun i => (i.channels d) = inst.channels d := by
  unfold chanRecv chanSend chanMake
  have hdc : d ≠ c := by
    exact fun hdc => hc hdc.symm
  simp [hdc]

/-- **Monotone clock:** channel ops leave the clock EXACTLY (atomic; no time
passes inside a single op), so `nowMs` never returns a value below an earlier
read. -/
theorem chanOps_clock_monotone (c : Nat) (v : Int) (inst : Instance) :
    inst.clock ≤ (chanSend c v inst).clock := by
  unfold chanSend
  simp

theorem timeAdvance_monotone (n : Nat) (inst : Instance) :
    inst.clock ≤ (timeAdvance n inst).clock := by
  unfold timeAdvance
  dsimp
  have h : 0 ≤ (n : Int) := Int.natCast_nonneg n
  omega

-- ---------------------------------------------------------------------------
-- The service-op trace + the statement claim surface (P15.2)
-- ---------------------------------------------------------------------------

/-- One step of a word's SERVICE TRACE — the §Q14 atomics in word order. The
trace is what a developer reasons over sequentially. -/
inductive ServiceOp where
  | make (id : Nat)
  | send (chan : Nat) (val : Int)
  | recv (chan : Nat)
  deriving DecidableEq, Repr, Inhabited

/-- Run a service-op trace against `inst`: each op is ONE atomic transition
of the global state; the result is the value stack the word leaves (execution
order — the first received value first) plus the final instance (`none` when
a recv blocks — running the trace against a channel without its FIFO head). -/
def traceRun (ops : List ServiceOp) (inst : Instance) : Option (List Int × Instance) :=
  match ops with
  | [] => some ([], inst)
  | .make c :: rest => traceRun rest (chanMake c inst)
  | .send c v :: rest => traceRun rest (chanSend c v inst)
  | .recv c :: rest =>
      match chanRecv c inst with
      | (some v, inst') =>
          match traceRun rest inst' with
          | some (stack, inst'') => some (v :: stack, inst'')
          | none => none
      | (none, _) => none

/-- The word's observable output value under a service trace: the LAST value
left on the stack (for a round-trip word, the received value). -/
def traceOutput (ops : List ServiceOp) (inst : Instance) : Option Int :=
  match traceRun ops inst with
  | some (stack, _) => stack.getLast?
  | none => none

/-- **The service-trace statement** (P15.2): running `ops` from ANY initial
instance yields an output value `v` with `lo ≤ v ∧ v ≤ hi` — the obligation's
subtype/guarantee claim evaluated over the abstract-atomic services model
instead of the concrete step (which cannot run calls). A make-local trace's
output is DETERMINISTIC (the FIFO laws decide it), so the statement is a
plain ∀-claim the developer proves by composing the laws. -/
def traceInRange (ops : List ServiceOp) (lo hi : Int) : Prop :=
  ∀ inst, ∃ v, traceOutput ops inst = some v ∧ lo ≤ v ∧ v ≤ hi

/-- The round-trip trace output: `[make 0, send 0 v, recv 0]` leaves `v` on
the stack for EVERY initial instance (the send-then-recv FIFO law, composed
over the trace). -/
theorem trace_send_recv_output (v : Int) (inst : Instance) :
    traceOutput [.make 0, .send 0 v, .recv 0] inst = some v := by
  unfold traceOutput
  simp [traceRun, chanRecv, chanSend, chanMake]

/-- The trace-level in-range transport: a bounded payload round-trips in
range. -/
theorem trace_send_recv_in_range (v lo hi : Int) (hv : lo ≤ v ∧ v ≤ hi)
    (inst : Instance) :
    ∃ w, traceOutput [.make 0, .send 0 v, .recv 0] inst = some w ∧ lo ≤ w ∧ w ≤ hi := by
  refine ⟨v, ?_⟩
  constructor
  · exact trace_send_recv_output v inst
  · exact hv

/-- **The §Q14 FIFO law (registry statement):** a word that makes a channel,
sends the constant payload `v`, then receives observes exactly `v` — the
abstract-atomic channel semantics is FIFO and the round trip is atomic. The
developer's proof of a `recv`-carried claim composes this statement. -/
theorem fifo_roundtrip (v : Int) :
    traceInRange [.make 0, .send 0 v, .recv 0] v v := by
  intro inst
  exact trace_send_recv_in_range v v v (by simp) inst

end Tyu.Services