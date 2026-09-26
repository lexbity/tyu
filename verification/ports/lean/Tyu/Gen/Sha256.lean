import Std

/-! SHA-256 (FIPS 180-4), hand-rolled for the Lean port (PLAN-VERIFY-3 P5).

The statement renderer (`Tyu/Gen/Render.lean`) computes `statement_hash` and
`word_ir_hash` *in Lean* — an independent implementation of the canonical
statement encoding + digest of `crates/verifier/src/stmt.rs`. The port gate
(`crates/tooling-tests/tests/gen_statement_drift.rs`) cross-checks the two
implementations over the corpus: any drift between the Rust encoder and the
Lean renderer is a byte-level failure (the renderer↔encoder drift lock,
P5.1 exit).

This module is *host-side, render-time only*: it is computation for the `gen`
executable and git-pinned golden files, never theorem material. FR-14 holds
here too: SHA-256 is the only digest this module constructs.

Integrity of this implementation is pinned by the known-answer vectors in
`gen --selfcheck` (`Tyu.Gen.Render.selfcheck`): "" → e3b0c442…, "abc" →
ba7816bf…, and the canonical-statement fixture chosen to match the Rust
encoder exactly. Values are the FIPS test vectors; the statement fixture is
computed once and cross-checked against the Rust encoder by
`crates/tooling-tests/tests/gen_statement_drift.rs`.
-/

namespace Tyu.Gen.Sha256

/-- A 32-bit word from a `Nat` literal, wrapping mod 2^32 (the constant
tables need values ≥ 2^31, which `OfNat` refuses; `UInt32.ofNat` wraps). -/
def w (n : Nat) : UInt32 := UInt32.ofNat n

/-- A byte from a `Nat`, wrapping mod 256. -/
def u8 (n : Nat) : UInt8 := UInt8.ofNat n

/-- Shift amounts are `UInt32`-typed in Lean's `HShiftRight` instances for
`UInt32` — a named form keeps the intent explicit. -/
def shl (x : UInt32) (n : Nat) : UInt32 := x <<< UInt32.ofNat n
def shr (x : UInt32) (n : Nat) : UInt32 := x >>> UInt32.ofNat n

/-- Rotate-right on 32-bit words. -/
def rotr (x : UInt32) (n : Nat) : UInt32 :=
  shr x n ||| shl x (32 - n)

/-- The 64 round constants. -/
def K : List UInt32 :=
  [ w 0x428a2f98, w 0x71374491, w 0xb5c0fbcf, w 0xe9b5dba5, w 0x3956c25b, w 0x59f111f1, w 0x923f82a4, w 0xab1c5ed5
  , w 0xd807aa98, w 0x12835b01, w 0x243185be, w 0x550c7dc3, w 0x72be5d74, w 0x80deb1fe, w 0x9bdc06a7, w 0xc19bf174
  , w 0xe49b69c1, w 0xefbe4786, w 0x0fc19dc6, w 0x240ca1cc, w 0x2de92c6f, w 0x4a7484aa, w 0x5cb0a9dc, w 0x76f988da
  , w 0x983e5152, w 0xa831c66d, w 0xb00327c8, w 0xbf597fc7, w 0xc6e00bf3, w 0xd5a79147, w 0x06ca6351, w 0x14292967
  , w 0x27b70a85, w 0x2e1b2138, w 0x4d2c6dfc, w 0x53380d13, w 0x650a7354, w 0x766a0abb, w 0x81c2c92e, w 0x92722c85
  , w 0xa2bfe8a1, w 0xa81a664b, w 0xc24b8b70, w 0xc76c51a3, w 0xd192e819, w 0xd6990624, w 0xf40e3585, w 0x106aa070
  , w 0x19a4c116, w 0x1e376c08, w 0x2748774c, w 0x34b0bcb5, w 0x391c0cb3, w 0x4ed8aa4a, w 0x5b9cca4f, w 0x682e6ff3
  , w 0x748f82ee, w 0x78a5636f, w 0x84c87814, w 0x8cc70208, w 0x90befffa, w 0xa4506ceb, w 0xbef9a3f7, w 0xc67178f2 ]

/-- σ0 and σ1 of the schedule. -/
def sigma0 (x : UInt32) : UInt32 := rotr x 7 ^^^ rotr x 18 ^^^ shr x 3
def sigma1 (x : UInt32) : UInt32 := rotr x 17 ^^^ rotr x 19 ^^^ shr x 10

/-- The initial hash state. -/
def H0 : List UInt32 :=
  [ w 0x6a09e667, w 0xbb67ae85, w 0x3c6ef372, w 0xa54ff53a, w 0x510e527f, w 0x9b05688c, w 0x1f83d9ab, w 0x5be0cd19 ]

/-- Big-endian word from four bytes. -/
def wordOfBytes (b0 b1 b2 b3 : UInt8) : UInt32 :=
  w ((b0.toNat <<< 24) + (b1.toNat <<< 16) + (b2.toNat <<< 8) + b3.toNat)

/-- Four big-endian bytes of a word. -/
def bytesOfWord (x : UInt32) : List UInt8 :=
  [ u8 ((shr x 24).toNat % 256), u8 ((shr x 16).toNat % 256), u8 ((shr x 8).toNat % 256), u8 (x.toNat % 256) ]

/-- The next schedule word given the running window `ws` (head = newest,
i.e. `ws.getD 0` = W[i-1] … `ws.getD 15` = W[i-16]). -/
def nextW (ws : List UInt32) : UInt32 :=
  sigma1 (ws.getD 1 0) + ws.getD 6 0 + sigma0 (ws.getD 14 0) + ws.getD 15 0

/-- The message schedule: W[0..63] (head = W[0]). -/
def schedule (block : List UInt8) : List UInt32 :=
  let rec chunk : List UInt8 → List UInt32
    | b0 :: b1 :: b2 :: b3 :: rest => wordOfBytes b0 b1 b2 b3 :: chunk rest
    | _ => []
    termination_by bs => bs.length
    decreasing_by
      simp_wf
      omega
  let w0 := chunk block
  let rec go (k : Nat) (acc : List UInt32) (out : List UInt32) : List UInt32 :=
    match k with
    | 0 => out
    | k' + 1 => go k' (nextW acc :: acc) (nextW acc :: out)
  -- acc grows head-first from W[15]..W[0]; after 48 rounds, `out` holds
  -- W[16]..W[63] head-first; the full schedule is W[0..15] ++ out.reverse.
  let full := go 48 w0.reverse []
  w0 ++ full.reverse

/-- The compression step: `[a, b, c, d, e, f, g, h]` under one round. -/
def compressStep (state : List UInt32) (wk : UInt32) : List UInt32 :=
  match state with
  | a :: b :: c :: d :: e :: f :: g :: h :: _ =>
      let s1 := rotr e 6 ^^^ rotr e 11 ^^^ rotr e 25
      let ch := (e &&& f) ^^^ ((~~~ e) &&& g)
      let t1 := h + s1 + ch + wk
      let s0 := rotr a 2 ^^^ rotr a 13 ^^^ rotr a 22
      let maj := (a &&& b) ^^^ (a &&& c) ^^^ (b &&& c)
      let t2 := s0 + maj
      [ t1 + t2, a, b, c, d + t1, e, f, g ]
  | _ => state

/-- One full block compression. -/
def compressBlock (state : List UInt32) (block : List UInt8) : List UInt32 :=
  let ws := schedule block
  let rec go : List UInt32 → List UInt32 → List UInt32 → List UInt32
    | [], _, acc => acc
    | w' :: rest, k :: ks, st => go rest ks (compressStep st (w' + k))
    | _, _, acc => acc
  let finished := go ws K state
  let rec zipAdd : List UInt32 → List UInt32 → List UInt32
    | [], _ => []
    | _ :: _, [] => []
    | x :: xs, y :: ys => (x + y) :: zipAdd xs ys
  zipAdd state finished

/-- Eight big-endian bytes of a `Nat` (the 64-bit length field). -/
def be64 (v : Nat) : List UInt8 :=
  let rec go : Nat → Nat → List UInt8
    | 0, _ => []
    | k + 1, v => u8 (v % 256) :: go k (v / 256)
  (go 8 v).reverse

/-- The SHA-256 padding: append `0x80`, zero bytes, then the 64-bit
big-endian message length (in bits). Total + correct on all inputs. -/
def pad (msg : List UInt8) : List UInt8 :=
  let n := msg.length
  let rem := n % 64
  -- p bytes of padding total, p ≥ 9 (1 × 0x80 + 8 × length), n + p ≡ 0 (mod 64)
  let p0 := 64 - rem
  let p := if p0 < 9 then p0 + 64 else p0
  let zeros := List.replicate (p - 9) 0
  msg ++ [128] ++ zeros ++ be64 (n * 8)

/-- Split a padded byte list into 64-byte blocks (total by construction:
`List.range` × drop/take). -/
def splitBlocks (bs : List UInt8) : List (List UInt8) :=
  let n := (bs.length + 63) / 64
  (List.range n).map (fun i => (bs.drop (i * 64)).take 64)

/-- SHA-256 of a byte list — 32 bytes, big-endian. -/
def sha256 (msg : List UInt8) : List UInt8 :=
  let blocks := splitBlocks (pad msg)
  let final := blocks.foldl (fun st b => compressBlock st b) H0
  final.flatMap bytesOfWord

/-- SHA-256 of a `String` over its code points (the canonical statement
bytes are pure ASCII by construction — every byte maps 1:1 to a code point;
a non-ASCII character would make the digest disagree with the Rust encoder,
which the drift test would catch — fail-closed by construction). -/
def sha256Str (s : String) : List UInt8 :=
  sha256 (s.toList.map (fun c => u8 (c.toNat % 256)))

/-- One byte as two lowercase hex characters. -/
def hexDigit (n : Nat) : Char :=
  match n % 16 with
  | 0 => '0' | 1 => '1' | 2 => '2' | 3 => '3' | 4 => '4' | 5 => '5' | 6 => '6' | 7 => '7'
  | 8 => '8' | 9 => '9' | 10 => 'a' | 11 => 'b' | 12 => 'c' | 13 => 'd' | 14 => 'e' | _ => 'f'

def hexOfByte (b : UInt8) : String :=
  String.ofList [hexDigit (b.toNat >>> 4), hexDigit b.toNat]

/-- Full-length lowercase hex of a digest (64 chars). -/
def toHex (bs : List UInt8) : String :=
  String.join (bs.map hexOfByte)

/-- The truncated 16-hex form (`<sha256:16>` of §6.1). -/
def toHex16 (bs : List UInt8) : String :=
  toHex (bs.take 8)

/-- `sha256_hex16` of a string (the `word_ir_hash` form). -/
def sha256Hex16 (s : String) : String :=
  toHex16 (sha256Str s)

/-- `statement_hash` (the 64-hex form of §6.2). -/
def sha256Hex (s : String) : String :=
  toHex (sha256Str s)

end Tyu.Gen.Sha256