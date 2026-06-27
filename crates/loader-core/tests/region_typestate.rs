#[test]
#[cfg_attr(miri, ignore)]
fn region_typestate_compile_failures() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/region_rw_entry.rs");
    t.compile_fail("tests/ui/region_use_after_release.rs");
    t.compile_fail("tests/ui/loader_platform_security_required.rs");
}
