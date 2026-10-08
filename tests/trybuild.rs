#[test]
fn only_the_pacer_mints_the_token_an_acquire_takes() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/visible_forged.rs");
    t.compile_fail("tests/ui/acquire_without_token.rs");
}
