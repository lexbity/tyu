//! Structured qualification report shared by runners and CI consumers.

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

use crate::CoverageAxis;

pub const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QualificationReport {
    pub schema_version: u32,
    pub selections: Vec<SelectionReport>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SelectionReport {
    pub label: String,
    pub target: String,
    pub rung_proven: String,
    pub advertised_capabilities: Vec<String>,
    pub ran: Vec<String>,
    pub skipped: Vec<SkippedFixture>,
    pub required_axes: Vec<CoverageAxis>,
    pub covered_axes: Vec<CoverageAxis>,
    pub uncovered_axes: Vec<CoverageAxis>,
    pub verdict: Verdict,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkippedFixture {
    pub name: String,
    pub reason: SkipReason,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkipReason {
    CapabilityMissing,
    FileMissing,
    PoisonUnsupported,
    QemuUnsupported,
    TargetUnsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Pass,
    Fail,
    /// Selection was not evaluated (e.g. hardware-rung pack excluded from
    /// emulator qualification).  Never counts as a failure in aggregate.
    #[serde(rename = "n/a")]
    NotApplicable,
}

pub fn render_human(report: &QualificationReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "qualification-report schema={}", report.schema_version);
    for selection in &report.selections {
        let _ = writeln!(
            out,
            "selection={} target={} rung={} verdict={}",
            selection.label,
            selection.target,
            selection.rung_proven,
            verdict_str(selection.verdict)
        );
        let _ = writeln!(
            out,
            "  advertised-capabilities={}",
            list_strings(&selection.advertised_capabilities)
        );
        let _ = writeln!(
            out,
            "  ran={} ({})",
            selection.ran.len(),
            list_strings(&selection.ran)
        );
        if selection.skipped.is_empty() {
            let _ = writeln!(out, "  skipped=0");
        } else {
            let _ = writeln!(out, "  skipped={}", selection.skipped.len());
            for skipped in &selection.skipped {
                let _ = writeln!(
                    out,
                    "    fixture={} reason={} detail={}",
                    skipped.name,
                    skip_reason_str(skipped.reason),
                    skipped.detail
                );
            }
        }
        let _ = writeln!(
            out,
            "  required-axes={}",
            list_axes(&selection.required_axes)
        );
        let _ = writeln!(out, "  covered-axes={}", list_axes(&selection.covered_axes));
        if selection.uncovered_axes.is_empty() {
            let _ = writeln!(out, "  uncovered-axes=[]");
        } else {
            let _ = writeln!(
                out,
                "  uncovered-axes={}",
                list_axes(&selection.uncovered_axes)
            );
            for axis in &selection.uncovered_axes {
                let _ = writeln!(out, "    uncovered-axis={}", axis.as_str());
            }
        }
        for reason in &selection.reasons {
            let _ = writeln!(out, "  reason={reason}");
        }
    }
    out
}

fn list_strings(values: &[String]) -> String {
    if values.is_empty() {
        return "[]".to_string();
    }
    format!("[{}]", values.join(","))
}

fn list_axes(values: &[CoverageAxis]) -> String {
    if values.is_empty() {
        return "[]".to_string();
    }
    let mut out = String::from("[");
    for (i, axis) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(axis.as_str());
    }
    out.push(']');
    out
}

fn skip_reason_str(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::CapabilityMissing => "capability-missing",
        SkipReason::FileMissing => "file-missing",
        SkipReason::PoisonUnsupported => "poison-unsupported",
        SkipReason::QemuUnsupported => "qemu-unsupported",
        SkipReason::TargetUnsupported => "target-unsupported",
    }
}

fn verdict_str(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Pass => "pass",
        Verdict::Fail => "fail",
        Verdict::NotApplicable => "n/a",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn report() -> QualificationReport {
        QualificationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            selections: vec![SelectionReport {
                label: "x86_64-unknown-none".to_string(),
                target: "x86_64-unknown-none".to_string(),
                rung_proven: "target".to_string(),
                advertised_capabilities: vec![],
                ran: vec!["arithmetic".to_string()],
                skipped: vec![SkippedFixture {
                    name: "channels".to_string(),
                    reason: SkipReason::CapabilityMissing,
                    detail: "missing: Channels".to_string(),
                }],
                required_axes: vec![CoverageAxis::Arith, CoverageAxis::Stack],
                covered_axes: vec![CoverageAxis::Arith],
                uncovered_axes: vec![CoverageAxis::Stack],
                verdict: Verdict::Pass,
                reasons: vec![],
            }],
        }
    }

    #[test]
    fn render_human_lists_skips_and_axes() {
        let rendered = render_human(&report());
        assert!(rendered.contains("selection=x86_64-unknown-none"));
        assert!(rendered.contains("fixture=channels reason=capability-missing"));
        assert!(rendered.contains("covered-axes=[arith]"));
        assert!(rendered.contains("uncovered-axis=stack"));
    }

    #[test]
    fn json_roundtrip() {
        let original = report();
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"capability-missing\""));
        let decoded: QualificationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn axis_order_snapshot_shape() {
        let rendered = list_axes(&[
            CoverageAxis::Arith,
            CoverageAxis::CallAbi,
            CoverageAxis::DeepStack,
        ]);
        assert_eq!(rendered, "[arith,call-abi,deep-stack]");
    }
}
