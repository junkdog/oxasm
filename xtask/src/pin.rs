//! The upstream pin: ASM 9.10.1, released on Maven Central (oxasm-port-plan §1, §8).
//!
//! Bumping the pin means changing this file and `ASM_VERSION` in `oracle/Oracle.java` together,
//! then re-importing T0 (`cargo xtask corpus t0`) and regenerating the goldens.

/// The released ASM version used as the reference implementation.
pub const ASM_VERSION: &str = "9.10.1";

/// The upstream git tag the T0 corpus and the vendored oracle sources are taken from.
pub const ASM_TAG: &str = "ASM_9_10_1";

/// The commit `ASM_TAG` points at.
pub const ASM_COMMIT: &str = "38c169b1";

/// The `org.ow2.asm` artifacts the oracle runs on, with the SHA-256 of their released jars.
pub const ASM_JARS: [(&str, &str); 5] = [
    (
        "asm",
        "ed825d10ab1399c8c0cb669e688cf0c8c82629b4c8399b58352b68e92ca10fcb",
    ),
    (
        "asm-tree",
        "3dfb0d5b6a106cd40b5b250e39935fbf2f927f4477546a5369a3ac609cf0506b",
    ),
    (
        "asm-analysis",
        "dede75a21306b65974ecd8f87114ff6970f09fb794157a4ca09ab25c888c2bfc",
    ),
    (
        "asm-commons",
        "6d0abefb7cbf972ea16edb37ec14835372505063a45f976ab7ea889ed9497895",
    ),
    (
        "asm-util",
        "1bb99d091fba2597dc6d51193e9bbcf0d8447e7ed96bd8f0198b18152f09655c",
    ),
];
