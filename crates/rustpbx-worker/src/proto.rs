// Shared generated protobuf code lives in `rustpbx-proto`. Re-export it so
// existing `crate::proto::{control,edge}` paths keep resolving unchanged.
// `edge` is re-exported for completeness even if not all of it is used here.
#[allow(unused_imports)]
pub use rustpbx_proto::{control, edge};
