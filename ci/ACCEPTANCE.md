# PLAN-VERIFY-3 acceptance matrix (P16.3)

Every §11 acceptance criterion maps 1:1 to a named gate. The always-run
(Rust/toolchain-free) legs execute under `bash ci/acceptance.sh`; the
toolchain-tier legs (Lean 4 needed) execute under the port job
(`ci/port.sh`, `ci/differential.sh`) and are recorded here.

| # | Criterion (§11) | Gate(s) | Tier | Last green |
|---|---|---|---|---|
| 1 | Worked example end-to-end (two-module proven build + deploy + `tyu cert verify`) | `tooling-tests::policy_proven`, `tyu::deploy_verify_policy`, `tyu::cert_verify`, port `TYU_PROOF_E2E` (+ `proven_automation_only` for the rederive-only leg) | Rust + port | 2026-09-28 (acceptance CR-01/01b; proof-e2e on the port job) |
| 2 | Tamper matrix fails closed (§11.2 items 1–6) | `tooling-tests::tamper_matrix`; port `tests/tamper/` negatives (E6418/E6419/E6420) | Rust + port | 2026-09-28 (acceptance CR-02; port negatives on the port job) |
| 3 | Statement goldens green on all four triples; encoder perturbation flips the gate | `tooling-tests::statement_goldens`, `tooling-tests::gen_render_drift` | Rust | 2026-09-28 (acceptance CR-03/03b) |
| 4 | Port CI: conformance corpus zero divergence; T-C/T-S/T-CL/T-D proven; axiom audit; `lean4checker` | `verifier::export_drift`; `ci/port.sh lean` (conformance × 4 triples, AxiomAudit, REVIEW.md §3 registry, P9.3 fragment, bundle corpora) | Rust + port | 2026-09-28 (`export_drift` green; port job per run) |
| 5 | Differential ≥10^5 exact agreement (Rust vs port re-derivation) | `verifier::rederive_differential` (committed corpus) + `ci/differential.sh lean` (≥10^5, NFR-3 budget) | Rust + port | 2026-09-28 (acceptance CR-05; `differential.sh` green in the NFR run) |
| 6 | Automation ≥90% loop-free auto-discharge, rate published (NFR-6) | `ci/automation-rate.json` baseline published; remeasure via `lake exe automation_rate` | published | baseline committed (informational process gate) |
| 7 | Determinism: two full pipeline runs byte-identical | `verifier::stmt_encoder` (determinism), `tyu::cert_assembly` (two deploys byte-identical index), golden double-regen discipline | Rust | 2026-09-28 (acceptance CR-07/07b) |
| 8 | Fuzz: 3 decode targets with corpus seeds; PR smoke + 24h nightly | `ci/fuzz.sh` (seeded smoke, targets `obl_v2_decode`/`verdicts_v2_decode`/`cert_index_decode`); nightly 24h job | Rust | 2026-09-28 (acceptance CR-08 smoke green; nightly out-of-band) |
| 9 | `proven` × unmodeled bundle: build + deploy fail E6510; `no-open` succeeds, manifest records `unmodeled` | `tooling-tests::unmodeled_pipeline`, `tyu::deploy_pairing`, `tyu::platform_model_lint` | Rust | 2026-09-28 (acceptance CR-09/09b) |
| 10 | All extended gates green on reference CI | `bash ci/guards.sh` (G24–G51) | Rust | 2026-09-28 (acceptance CR-10) |

## NFR numbers (recorded from `bash ci/acceptance.sh --measure-nfr`)

| NFR | Definition | Gate | Recorded |
|---|---|---|---|
| NFR-1 | p95 per-module SVS-inclusive emission ≤ 250 ms | `ci/nfr.sh` timing over `ci/verify-corpus` | 2026-09-28: p95 = 10 ms (max 11 ms; n=20) — within gate |
| NFR-3 | Zero false-`Discharged` over ≥10^5 generated programs per CI run | `ci/differential.sh` (FR-13) | 2026-09-28: 100 000 programs, exact agreement, within budget |
| NFR-4 | 0 id changes under unrelated edits | `tooling-tests::obl_id_stability` | 2026-09-28: green |
| NFR-5 | `.obl.json` ≤ max(4 × `.asm`, 64 KiB); read cap 16 MiB | `ci/nfr.sh` size pass | 2026-09-28: worst = Contract 4819 B, within max(4×asm, 64 KiB) |

> P16.3 fuzz catch (2026-09-28, fixed in-tree): `read_obl` livelocked on a
> zero-width UTF-8 sequence (invalid lead byte 0xA1 inside a string) — the
> fuzzer's `obl_v2_decode` went from 0 exec/s to ~23k exec/s after the fix;
> pinned by `verifier/tests/codec_roundtrip.rs::invalid_utf8_lead_byte…`.
>
> Update the `Recorded` cells after each measurement/CI run. The numbers are
> the *published* state (P16.3: "NFR-1/3/4/5 verified with numbers").