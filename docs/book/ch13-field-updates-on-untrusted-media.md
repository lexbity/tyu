# Chapter 13 — Field Updates on Untrusted Media

> Anything that can update itself can be replaced by something else.
> The question is who gets the last word.
>
> — workshop wall

## 13.1 The Practice

> **The Practice.** *A module is safe to load when everything it claims has
> been checked or authenticated, and the loader refuses everything else —
> so the daring operation, installing new code on a running device from
> untrusted media, becomes a checklist the machine executes.*

This is the capstone. Ten chapters of claims — types, contracts, effects,
bounds, locks, interfaces — were never just compile-time comforts. They
were accumulating into a list that a *loader* can hold a module to, at
load time, on a device, from media the device's owners do not control.
This chapter is where the list is read out loud.

## 13.2 The two-image world

The field-update architecture has two players:

- **The base image** — the proven program: vectors, runtime, the loader,
  the interrupt regime. Chapter 11's cross-context rule closed over
  exactly this program, which is why the rule could be whole-program at
  all.
- **The field module** — a `.lmod` container that arrives on media of
  uncertain parentage and, if every gate passes, computes under the base
  image's regime.

And the rule from chapter 11 returns with its reason attached: **a loaded
module may not install an interrupt handler** (the static-ISR rule). The
cross-context check needed to see every reacher of a resource at once; a
module that added a handler at load time would add reachability after the
proof closed. Loaded code *computes* under the interrupt regime the base
proved; it cannot change the regime. Swapping a handler's behavior is
still possible — through a statically declared trampoline word that calls
into loaded code — which keeps the proof closed and the door open.

## 13.3 The container

`lmod-pack` turns a compiled module into the loadable container. Lab 13.1
packs this book's smallest complete module:

```tyu
# labs/ch13/green-01-ping.mod — Lab 13.1 (green)
# The field module: a minimal word-set whose .lmod will be packed, signed,
# and (in Lab 13.2) loaded into running firmware through the loader.
# Expected: packs and signs cleanly with the chapter's tool transcript.
module Ping;
import platform/testio { testio.write-byte };
: main ( -- i64 ) 83 testio.write-byte 10 testio.write-byte 0 ;
export { main };
end;
```

```console
$ tyu build labs/ch13/green-01-ping.mod --mode=dynamic \
      --target=x86_64-unknown-none --platform=x86_64-unknown-none \
      --sysroot=sysroot --out-dir=build/ch13
$ ls build/ch13
Ping.lmod   Ping-f2c68787dd6510af.o   … image.elf
```

The `.lmod` layout is fixed and little-endian: a header, the module's
contract record (`.lang.modinfo` — the interface of chapter 10, compiled),
code, read-only and writable data, the relocation table, and a signature
trailer. Two properties matter more than the byte layout. **Every offset
and length is validated before use** — a corrupted container is a clean
rejection, never a wild read. And the **signature covers the signed
region**, which includes the header and its `abi_hash` — the attacker
cannot edit the compatibility claim without breaking the authenticity
check that guards it.

## 13.4 The gate: abi_hash

The first gate is the one this book has been quietly building since
chapter 4. Each word's interface — stack effect, effects, capabilities,
stack bound — is folded, with the target's calling convention and the
runtime's ABI revision, into a 64-bit `abi_hash`. The module's hash and
the base image's expected hash must match exactly, or the module is
refused (E5200, Lab 13.2's negative suite). There is no negotiation, no
"best effort" compatibility: divergence is *forced into the hash* so it
cannot be silently ignored. Chapter 10's `.def` was the promise; the hash
is the promise's fingerprint, and the loader compares fingerprints.

## 13.5 Trust tiers: integrity, authenticity, confidentiality

The loader distinguishes three properties, checked in layers — the trust
tiers:

- **Tier 0 — integrity.** Every container field is bounds-validated and
  the module is placed W^X, transactionally: any failure leaves no
  half-loaded state. This is the floor; a base image with secure boot
  needs nothing more for its own baked-in code.
- **Tier 1 — authenticity.** The signed region is verified against a
  signature (HMAC-SHA256 in v1) before anything from it is trusted.
  Signing happens at build time — `--metal-sign-key` — and Lab 13.1
  shows the tool transcript:

```console
$ lmod-sign build/ch13/Ping.lmod build/ch13/Ping.signed.lmod \
      --key=abab…ab
signed /tmp/tyu_ch13/pkg/Ping.lmod -> /tmp/tyu_ch13/pkg/Ping.signed.lmod
    (scheme=HMAC-SHA256, 705 bytes)
```

- **Tier 2 — re-derivation.** The strongest tier: the loader re-derives
  the *stack bound* from the signed instruction stream itself, rather
  than trusting the carried number — chapter 6's monoid, recomputed by
  the untrusting party.

Confidentiality is orthogonal and layered *on top of* authenticity:
ChaCha20-Poly1305 encryption, applied after signing, with the content key
wrapped per device or per fleet; the loader decrypts only after the
signature verifies. **Authenticity before confidentiality** — the loader
never decrypts data whose origin it has not established. Lab 13.3 runs
the encrypted-load suite.

## 13.6 The refusals

The negative suite is the chapter's red lab, and it inverts the usual
polarity: **each of its ten tests is an attempted crime, and the suite
passes because every crime is refused.** The verified refusal table:

| Attempted crime                                    | Loader verdict |
|-----------------------------------------------------|----------------|
| module built for a different ABI                    | E5200 |
| corrupted container (bad length/offset)             | E5201 |
| module declares an interrupt handler                | E5203 |
| unsupported relocation                              | E5204 |
| unresolved import                                   | E5205 |
| aperture base mismatch                              | E5219 |
| platform hash mismatch                              | E5220 |
| aperture table unresolved / malformed               | E5222 / E5223 |
| modinfo from a newer format version                 | E5224 |

Read row three again — the static-ISR rule from chapter 11, enforced at
load time.

> **Afterword (chapter 14):** the loader grows one more gate after this
> book's capstone — the `verify_manifest`, carrying each module's proof
> verdicts and a `proven` admission policy that can refuse any module
> whose obligations are not discharged or runtime-checked. Chapter 14
> builds that machinery and the theorems it consumes. The rows around it are chapter 10 (interface identity), the
layout discipline of §13.3, and the platform descriptor of chapter 8
(apertures), each one re-checked by the untrusting party.

## 13.7 The threat model, briefly

What the loader defends: an attacker who controls the *update medium* —
a substituted, truncated, corrupted, or deliberately crafted module — can
neither execute unverified code (authenticity), nor confuse the loader
with malformed structure (integrity, always on), nor read the payload
(confidentiality, when deployed), nor smuggle a different ABI (the hash
gate), nor change the interrupt regime (the static-ISR rule).

What it does not defend, stated plainly. HMAC is *symmetric*: the device
that verifies holds the key, so the signing key's custody is the real
perimeter, and public-key signatures — verify without hold — are the
roadmap's known direction. A physically present attacker who extracts
the key defeats authenticity for future updates; the defenses assume key
custody. Load-once means a module cannot be swapped underneath a running
system — but also that updating means restarting. And none of this
extends the chapter 6 boundary: the loaded module's *native* stack is as
unproven as the base image's. The loader checks every claim a module
carries; it cannot check claims nobody wrote down.

## 13.8 Why this is the capstone

Count the claims a signed module carries, and see the whole book in one
container: its words' stack effects and types (chapters 2–3), its
contracts and effect labels (4–5), its data-stack bound (6), the
borrow-discipline of everything it links against (7), its interface
record mirrored from its `.def` (10), its reachability fixed by the
static-ISR rule (11). The loader's job is to refuse anything whose claims
are missing, mismatched, or unauthentic — and the reason that refusal can
be a *checklist* rather than a research project is that every chapter
before this one made one claim precise enough to appear on the list.
Nobody sane ships field-updatable firmware written in a language where
none of those claims exist. The daring was licensed, chapter by chapter,
by the rigor.

---

## Labs — Chapter 13

### Lab 13.1 — Pack and sign *(green)*

Fixture: `labs/ch13/green-01-ping.mod` (§13.3). Build it in dynamic mode
against the QEMU platform, then sign the container:

```console
$ tyu build labs/ch13/green-01-ping.mod --mode=dynamic \
      --target=x86_64-unknown-none --platform=x86_64-unknown-none \
      --sysroot=sysroot --out-dir=build/ch13
$ lmod-sign build/ch13/Ping.lmod build/ch13/Ping.signed.lmod \
      --key=ababababababababababababababababababababababababababababababab
signed build/ch13/Ping.lmod -> build/ch13/Ping.signed.lmod
    (scheme=HMAC-SHA256, 705 bytes)
```

**Notice** the signing tool's report: the scheme is stated on the wire,
not assumed — a verifier that cannot do HMAC-SHA256 refuses rather than
guesses.

### Lab 13.2 — The signed module loads *(green)*

```console
$ cargo test -p execution-tests --test dynamic_signed
test signed_dynamic_lmod_runs_under_qemu ... ok

test result: ok. 2 passed; 0 failed
```

The suite builds a signed module (the same signing key this chapter
uses), boots the base image under QEMU, and loads the module through the
in-firmware loader: ABI gate, signature verification, relocation, symbol
resolution, entry call — the module's marker reaches the host. This is
the field update, end to end, on the emulated board.

### Lab 13.3 — Encrypted at rest *(green)*

```console
$ cargo test -p execution-tests --test dynamic_encrypted
test result: ok. 4 passed; 0 failed
```

ChaCha20-Poly1305 encryption applied *after* signing; the loader
verifies the signature first, then unwraps the content key (per-device
or per-fleet, via the platform's own key custody) and decrypts the
sections. Four passed covers the encrypt/load paths and their
self-consistency checks.

### Lab 13.4 — Ten crimes, ten refusals *(red, the loader's own)*

```console
$ cargo test -p execution-tests --test dynamic_negative
test dynamic_isr_declaration_traps_5203 ... ok
test dynamic_unresolved_symbol_traps_5205 ... ok
test dynamic_unsupported_reloc_traps_5204 ... ok
    …
test result: ok. 10 passed; 0 failed
```

**Why:** each test attempts one crime from §13.6's table — a tampered
container, a module from a different ABI, a module that declares an
interrupt handler, a modinfo from the future — and asserts the exact
loader verdict. The suite *passes* when every crime is refused; that
inversion is the loader's whole personality. The star for this book's
narrative is E5203: chapter 11 proved the interrupt regime static, and
here is the loader holding the door.

### Post-mortem — the checksum is not a signature *(optional — for readers with C or embedded scars)*

The C update story this chapter refuses: a raw binary and a CRC — "we
verify integrity before flashing." A CRC defends against corruption, and
corruption was never the attacker. The crafted image — valid CRC,
relocated payload, a hook in the UART receive path — verifies perfectly,
because a checksum verifies *bits*, not *provenance*. Its cousin: the
update that includes a "compatibility version field" the bootloader
checks with `>=` — and every future module, including the incompatible
ones, sails through.

The layered answer maps each threat to a distinct mechanism: bounds
before use (corruption), signature over the signed region with the
compatibility hash inside it (forgery and drift), no decryption before
authentication (oracle avoidance), and the static-ISR rule (regime
change). Each is a *refusal with a code* — and the negative suite is the
proof that the refusals fire, not a design document hoping they would.

### From the workbench

Why refuse rather than negotiate? Two architectures existed at design
time: negotiate (the module and runtime compare versions and agree on a
compatible subset) or self-guard (any divergence in the claims that
matter changes a hash, and mismatched hashes are refused). Negotiation
code is itself code — it can have bugs, and it runs *before* the module
is trusted, which makes it the ideal first target. Self-guarding moves
the decision to a fingerprint comparison: there is no compatibility
subset to argue about, because the module was built against a runtime,
and this is either that runtime or it is not. The brittleness is
admitted — chapter 10's interface drift shows the same face — but
brittle-and-total beats flexible-and-arguable when the subject is what
code may run on a device in the field.

### Exercises

1. **Break the seal.** Take Lab 13.1's signed container and flip one byte
   inside the code region. Before checking: which property does the flip
   attack (integrity, authenticity, or confidentiality), and which check
   fires first? Then confirm with Lab 13.2's suite — flip the byte in its
   signed fixture and watch the negative suite's tamper case refuse it.
2. **Why inside the seal.** The `abi_hash` lives *inside* the signed
   region. Explain in two sentences why that placement matters: what
   could an attacker do with a module whose hash they could edit without
   breaking the signature?
3. **The reservation, reversed.** Chapter 10 let a module *reserve*
   effects in its interface. The loader's hash discipline means a
   reserved-effect module and the implemented one are different modules
   to any consumer. Is that the price of the hash discipline, or its
   point? Answer in three sentences using the words *promise*,
   *consumer*, and *field*.

---

*Next: chapter 14 — Tyu's Effect on Thinking. The epilogue: the style
distilled, the vigilance trap closed, and the map from checked claims to
provable ones.*
