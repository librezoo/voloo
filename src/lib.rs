//! Voloo Host: local software for the USB-C Voloo Switch.
// `OpError` carries details and faults for users and agents. Errors are rare and every
// call already costs a serial round trip, so boxing would only add noise.
#![allow(clippy::result_large_err)]
pub mod client;
pub mod config;
pub mod daemon;
pub mod device;
pub mod discover;
pub mod home;
pub mod link;
pub mod manager;
pub mod mcp;
pub mod observe;
pub mod protocol;
pub mod report;
pub mod sim;
