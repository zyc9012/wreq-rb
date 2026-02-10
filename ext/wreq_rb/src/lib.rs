#![allow(unused_imports)]

mod client;
mod error;
mod response;

use magnus::prelude::*;

/// Initialize the native extension.
/// This is called when `require "wreq_rb/wreq_rb"` is invoked.
#[magnus::init]
fn init(ruby: &magnus::Ruby) -> Result<(), magnus::Error> {
    let module = ruby.define_module("Wreq")?;

    error::init(ruby, &module)?;
    response::init(ruby, &module)?;
    client::init(ruby, &module)?;

    Ok(())
}
