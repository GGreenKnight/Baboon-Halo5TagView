//! Adapter from Halo 5 storage to the application's ordinary tag catalog.
use super::*;
mod document;
pub(crate) use document::{Halo5Backing, LoadedHalo5Tag, read_halo5_tag};

pub(crate) fn halo5_reference_entry(
    source: &blam_tags::halo5::Source,
    group: u32,
    name: &str,
) -> Result<(Arc<blam_tags::halo5::Source>, TagEntry)> {
    let group = blam_tags::format_group_tag(group);
    let (source, index) = source.with_loose_reference(&group, name)?;
    let e = &source.entries[index];
    let entry = TagEntry {
        key: format!("halo5:{index}:{}", e.name),
        display_path: e.name.clone(),
        group_tag: blam_tags::parse_group_tag(&e.group).unwrap_or(0),
        group_name: Some(e.group_name.clone()),
        location: TagEntryLocation::Halo5 { index },
    };
    Ok((Arc::new(source), entry))
}

pub fn load_halo5(path: PathBuf, definitions: &Path) -> Result<LoadedSourceData> {
    let source = Arc::new(blam_tags::halo5::Source::open(
        &path,
        blam_tags::halo5::Definitions::load(definitions)?,
    )?);
    let names = TagNameIndex::from_group_names(source.definitions.names().filter_map(|name| {
        let group = source.definitions.group_for_name(name)?;
        Some((blam_tags::parse_group_tag(&group)?, name.to_owned()))
    }));
    let entries: Vec<_> = source
        .entries
        .iter()
        .enumerate()
        .map(|(index, e)| TagEntry {
            key: format!("halo5:{index}:{}", e.name),
            display_path: e.name.clone(),
            group_tag: blam_tags::parse_group_tag(&e.group).unwrap_or(0),
            group_name: Some(e.group_name.clone()),
            location: TagEntryLocation::Halo5 { index },
        })
        .collect();
    let label = path
        .file_name()
        .and_then(|p| p.to_str())
        .unwrap_or("Halo 5: Guardians")
        .to_owned();
    Ok(LoadedSourceData {
        label,
        tree: build_tree(&entries),
        group_tree: build_group_tree(&entries),
        source: TagSource::Halo5 { source },
        names,
        game: None,
        entries,
        all_entries: Vec::new(),
        reverse_dependencies: None,
        initial_tag: None,
        key_hints: Default::default(),
        complete_scan: true,
        chosen_kit_layout: None,
    })
}
