//! The fork API lives in the Android-only `manatan_android` crate. Including
//! the same source here lets `cargo test -p fork-api-host-tests` exercise it
//! on a development machine; Android-only calls (JNI, the document scanner and
//! the keystore) are compiled but never reached by the tests.
#![allow(dead_code)]

#[path = "../../../bin/manatan_android/src/fork_api.rs"]
mod fork_api;
