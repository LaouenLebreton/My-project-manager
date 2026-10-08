//! Microsoft Project XML (MSPDI) import and export (spec 4.8).
//!
//! Import never drops data silently: everything it cannot represent is listed in an
//! [`ImportReport`], grouped as imported, approximated (with how) or ignored. Fields that
//! Super PM calculates itself, such as dates and slack, are listed as recalculated.
//!
//! The crate is pure: ids come from a caller-supplied generator.

mod export;
mod import;
mod report;
mod xml;

pub use export::export_mspdi;
pub use import::{import_mspdi, ImportError};
pub use report::{ImportReport, ReportLine};

/// MSPDI's XML namespace.
pub const NAMESPACE: &str = "http://schemas.microsoft.com/project";

#[cfg(test)]
mod tests;
