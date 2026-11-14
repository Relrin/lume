//! The host suite, with the host as a task in this process over an in-memory pipe.

mod suite;
mod support;

suite::host_suite!(support::InProcess);
