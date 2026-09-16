//! SEC-11: the `local-clock` build, which accepts renewal periods of seconds,
//! declares its own program id. Anchor refuses to execute a program whose
//! declared id differs from the address it was deployed to, so the
//! accelerated artifact cannot run at the public address even if uploaded.

#[allow(dead_code)]
const PUBLIC_ID: &str = "GkZ9HQvNe1m1KDPA3D9HtFWNdkMe2baaed2fKH8w4FUv";
#[allow(dead_code)]
const LOCAL_CLOCK_ID: &str = "CnA1TVJUnVLzh5FgWwNcNcdT6MdiTRKGgkudHihUHVun";

#[cfg(not(feature = "local-clock"))]
#[test]
fn the_standard_build_declares_the_public_program_id() {
    assert_eq!(lumi_support::ID.to_string(), PUBLIC_ID);
}

#[cfg(feature = "local-clock")]
#[test]
fn the_local_clock_build_declares_its_own_program_id() {
    assert_eq!(lumi_support::ID.to_string(), LOCAL_CLOCK_ID);
    assert_ne!(lumi_support::ID.to_string(), PUBLIC_ID);
}
