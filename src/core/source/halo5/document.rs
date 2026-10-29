//! Wrap the engine's Halo 5 TagFile in the application's shared document model.
use crate::core::document::{DocumentAccess, TagDocument};
pub(crate) use blam_tags::halo5::TagBacking as Halo5Backing;
use blam_tags::halo5::{Source, TagBacking};

pub(crate) struct LoadedHalo5Tag {
    pub(crate) document: TagDocument,
    pub(crate) backing: TagBacking,
}

pub(crate) fn read_halo5_tag(source: &Source, index: usize) -> anyhow::Result<LoadedHalo5Tag> {
    let loaded = blam_tags::halo5::read_tag(source, index)?;
    let writable_fields = loaded
        .backing
        .writable_fields()
        .map(str::to_owned)
        .collect();
    let mut document = TagDocument::clean(loaded.tag);
    document.access = DocumentAccess {
        writable_fields: Some(writable_fields),
        fixed_blocks: true,
        fields_only: true,
    };
    Ok(LoadedHalo5Tag {
        document,
        backing: loaded.backing,
    })
}
