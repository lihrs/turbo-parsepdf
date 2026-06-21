//! Map core parse faults to N-API errors.

use turbo_parsepdf_core::TurboParsePdfError;

/// Convert a fatal core error into a thrown N-API error carrying the stable code.
pub fn to_napi(e: TurboParsePdfError) -> napi::Error {
    napi::Error::new(
        napi::Status::GenericFailure,
        format!("{}: {}", e.code.as_str(), e.message),
    )
}
