use magnus::{ExceptionClass, Module};

static mut WREQ_ERROR: Option<ExceptionClass> = None;

pub fn wreq_error() -> ExceptionClass {
    unsafe { WREQ_ERROR.unwrap() }
}

pub fn init(ruby: &magnus::Ruby, module: &magnus::RModule) -> Result<(), magnus::Error> {
    let error_class = module.define_error("Error", ruby.exception_standard_error())?;
    unsafe {
        WREQ_ERROR = Some(error_class);
    }
    Ok(())
}

/// Convert a wreq::Error into a magnus::Error
pub fn to_magnus_error(err: wreq::Error) -> magnus::Error {
    magnus::Error::new(wreq_error(), err.to_string())
}

/// Convert any Display error into a magnus::Error
pub fn generic_error(msg: impl std::fmt::Display) -> magnus::Error {
    magnus::Error::new(wreq_error(), msg.to_string())
}
