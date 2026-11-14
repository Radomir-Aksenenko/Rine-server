pub use rine_data::items::ItemInfo;

pub fn info(item: u32) -> Option<&'static ItemInfo> {
    rine_data::get().items.get(item)
}

pub fn id(name: &str) -> Option<u32> {
    rine_data::get().items.by_name(name).map(|i| i.id)
}

/// Default block state placed by this item (0 = not a block item).
pub fn places(item: u32) -> u16 {
    rine_data::get().items.places.get(item as usize).copied().unwrap_or(0)
}
