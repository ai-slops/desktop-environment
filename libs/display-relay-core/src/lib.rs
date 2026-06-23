//! Shared display-topology and coordinate-mapping primitives for relay apps.

mod geometry;
mod session;

pub use geometry::{DisplayArea, PointerSample, VirtualDesktop};
pub use session::{CaptureTarget, RelayConfig};
