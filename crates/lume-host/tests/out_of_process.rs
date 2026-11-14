//! The host suite, with the host as a process of its own behind the per-user endpoint.

mod suite;
mod support;

suite::host_suite!(support::OutOfProcess);
