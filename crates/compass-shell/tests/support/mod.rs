//! Shared test support for the mock-bus suite.
//!
//! Each integration test is its own crate, so this module is compiled once per
//! test binary and every binary sees the whole of it. `mock_bus.rs` does not
//! use the introspection comparator and `contract_introspection.rs` does not
//! use the malformed-reply mocks, so under `-D warnings` each binary would
//! reject the other's helpers as dead code. The lint is right about the
//! compilation unit and wrong about the crate, so it is relaxed here — and
//! only here, over test scaffolding, never over shipped code.
#![allow(dead_code)]

pub mod bus;
pub mod introspect;
pub mod mock;
