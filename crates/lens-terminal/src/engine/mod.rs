pub mod frame;

#[expect(
    unused_imports,
    reason = "public engine surface; consumed as later tasks land"
)]
pub use frame::{CellStyle, Frame, FrameCell, FrameRow, Rgb, UnderlineStyle};
