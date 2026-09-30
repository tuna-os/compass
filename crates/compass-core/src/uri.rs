//! Percent-encoding sets, for the `percent-encoding` crate.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};

/// RFC 3986's unreserved characters stay as they are; everything else is
/// escaped. What `QUrl::toPercentEncoding` does with no exceptions.
pub const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// [`UNRESERVED`], plus `/` and `:`: an image URL's query keeps its paths and
/// schemes readable.
pub const QUERY_VALUE: &AsciiSet = &UNRESERVED.remove(b'/').remove(b':');
