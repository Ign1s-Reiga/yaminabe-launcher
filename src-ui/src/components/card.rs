pub mod instance_card;
pub mod link_notice_card;
pub mod managed_notice_card;
pub mod pack_tile;
pub mod result_card;

/// How a collection of cards is laid out: full-width rows, or a grid of tiles.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Layout {
    List,
    #[default]
    Cards,
}
