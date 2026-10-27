//! HUD message index safety belongs to the editor, not the tag serializer.
use super::*;
use crate::core::source::LoadedSourceData;
use blam_tags::string_list::{self as strings, StringEntry, StringListEdit};
use std::collections::BTreeSet;

pub(in crate::app) const EXPERT_REQUIRED: &str = "Global HUD prompts use fixed indexes. Operations that change message slots require Expert Mode.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum HudMessageUsage {
    Global,
    Mission,
    #[default]
    Unknown,
}

impl HudMessageUsage {
    pub(in crate::app) fn locked(self, expert: bool) -> bool {
        self == Self::Global && !expert
    }
    pub(in crate::app) fn warning(self) -> &'static str {
        match self {
            Self::Global => {
                "Global HUD prompts use fixed indexes. Changing entry order can make the game display the wrong prompts. Operations that change message slots require Expert Mode."
            }
            Self::Mission => {
                "Mission scripts reference these messages by compiled index. After reordering, inserting, deleting or renaming entries, recompile the referencing scenario's scripts before testing or rebuilding the map. Text-only edits do not require script recompilation."
            }
            Self::Unknown => {
                "HUD message usage could not be determined. Global prompts may use fixed indexes; mission scripts use compiled indexes. Preserve global prompt order, and recompile referencing scenario scripts after reordering, inserting, deleting or renaming mission messages."
            }
        }
    }
}

fn normalized(path: &str) -> String {
    path.trim_end_matches('\0')
        .replace('/', "\\")
        .to_ascii_lowercase()
}
fn reference(tag: &TagFile, field: &str, group: u32) -> Option<String> {
    match tag.root().field_path(field)?.value()? {
        TagFieldData::TagReference(value) => value
            .group_tag_and_name
            .filter(|(g, _)| *g == group)
            .map(|(_, p)| p),
        _ => None,
    }
}

/// Consult references, never the HUD message file's name. Open documents take
/// precedence over cached on-disk tags, including unsaved HUD-globals changes.
pub(in crate::app) fn usage(
    source: &LoadedSourceData,
    key: &str,
    open: &HashMap<String, TagDocument>,
) -> HudMessageUsage {
    let Some(entry) = source.entry_for_key(key) else {
        return HudMessageUsage::Unknown;
    };
    if entry.group_tag != strings::HUD_GROUP {
        return HudMessageUsage::Unknown;
    }
    let target = normalized(
        entry
            .display_path
            .strip_suffix(".hud_message_text")
            .unwrap_or(&entry.display_path),
    );
    let hudg = u32::from_be_bytes(*b"hudg");
    let scnr = u32::from_be_bytes(*b"scnr");
    let refers = |tag: &TagFile| {
        reference(tag, "hud messages", strings::HUD_GROUP).is_some_and(|p| normalized(&p) == target)
    };
    let mut mission = false;
    // Include loaded documents even before the background catalog scan finishes.
    for doc in open.values() {
        if doc.tag.group().tag == hudg && refers(&doc.tag) {
            return HudMessageUsage::Global;
        }
        if doc.tag.group().tag == scnr && refers(&doc.tag) {
            mission = true;
        }
    }
    let read = |group, path: &str| fields::string_editors::hud_reference_tag(source, group, path);
    // The active globals path also works before any HUD-globals folder is expanded.
    let matg = u32::from_be_bytes(*b"matg");
    let open_globals = source
        .all_entries
        .iter()
        .chain(&source.entries)
        .find(|e| e.group_tag == matg && normalized(&e.display_path) == "globals\\globals.globals")
        .and_then(|e| open.get(&e.key));
    let disk_globals = if open_globals.is_none() {
        read(matg, "globals\\globals")
    } else {
        None
    };
    if let Some(globals) = open_globals.map(|doc| &doc.tag).or(disk_globals.as_deref()) {
        if let Some(path) = reference(globals, "interface bitmaps[0]/hud globals", hudg) {
            let open_hud = source
                .all_entries
                .iter()
                .chain(&source.entries)
                .find(|e| {
                    e.group_tag == hudg
                        && normalized(&e.display_path)
                            == format!("{}.hud_globals", normalized(&path))
                })
                .and_then(|e| open.get(&e.key));
            let matches = match open_hud {
                Some(doc) => refers(&doc.tag),
                None => read(hudg, &path).is_some_and(|tag| refers(&tag)),
            };
            if matches {
                return HudMessageUsage::Global;
            }
        }
    }
    for entry in source
        .all_entries
        .iter()
        .chain(&source.entries)
        .filter(|entry| entry.group_tag == hudg)
    {
        if let Some(doc) = open.get(&entry.key) {
            if refers(&doc.tag) {
                return HudMessageUsage::Global;
            }
        } else {
            let path = entry
                .display_path
                .strip_suffix(".hud_globals")
                .unwrap_or(&entry.display_path);
            if read(hudg, path).is_some_and(|tag| refers(&tag)) {
                return HudMessageUsage::Global;
            }
        }
    }
    if let Some(index) = &source.reverse_dependencies {
        mission |= index
            .dependents_for(strings::HUD_GROUP, &target)
            .iter()
            .any(|key| {
                source
                    .entry_for_key(key)
                    .is_some_and(|e| e.group_tag == scnr)
            });
    }
    // A scenario alongside a mission HUD tag is checked, rather than inferred
    // from its folder or filename. No full scan of large scenario bodies.
    let folder = target.rsplit_once('\\').map(|(folder, _)| folder);
    for entry in source
        .all_entries
        .iter()
        .chain(&source.entries)
        .filter(|e| {
            e.group_tag == scnr
                && normalized(&e.display_path)
                    .rsplit_once('\\')
                    .map(|(f, _)| f.to_owned())
                    .as_deref()
                    == folder
        })
    {
        let path = entry
            .display_path
            .strip_suffix(".scenario")
            .unwrap_or(&entry.display_path);
        let matches = match open.get(&entry.key) {
            Some(doc) => refers(&doc.tag),
            None => read(scnr, path).is_some_and(|tag| refers(&tag)),
        };
        mission |= matches;
    }
    if mission {
        HudMessageUsage::Mission
    } else {
        HudMessageUsage::Unknown
    }
}

pub(in crate::app) fn same_slots(before: &[StringEntry], after: &[StringEntry]) -> bool {
    before.len() == after.len() && before.iter().zip(after).all(|(a, b)| a.id == b.id)
}

impl Baboon {
    pub(in crate::app) fn hud_message_usage(&self, index: usize, key: &str) -> HudMessageUsage {
        let kit = &self.model.kits[index];
        kit.source
            .as_ref()
            .map_or(HudMessageUsage::Unknown, |source| {
                usage(source, key, &kit.parsed_tags)
            })
    }
    pub(in crate::app) fn check_hud_rows(
        &self,
        index: usize,
        key: &str,
        before: &[StringEntry],
        after: &[StringEntry],
    ) -> Result<(), String> {
        if self
            .hud_message_usage(index, key)
            .locked(self.model.prefs.expert_mode)
            && !same_slots(before, after)
        {
            Err(EXPERT_REQUIRED.to_owned())
        } else {
            Ok(())
        }
    }
    pub(in crate::app) fn check_hud_ops(
        &self,
        index: usize,
        key: &str,
        ops: &DeferredOps,
    ) -> Result<(), String> {
        if !self
            .hud_message_usage(index, key)
            .locked(self.model.prefs.expert_mode)
        {
            return Ok(());
        }
        if ops
            .block_ops
            .iter()
            .any(|op| op.path == "messages" || op.path == "message elements")
        {
            return Err(EXPERT_REQUIRED.to_owned());
        }
        let Some(mut list) = self.model.kits[index]
            .parsed_tags
            .get(key)
            .and_then(|doc| strings::read_string_list(&doc.tag))
        else {
            return Ok(());
        };
        let before = list.entries.clone();
        // Simulate only slot identity; text edits and name edits in place are
        // permitted. Replace imports may remove/reinsert the same ordered names.
        for op in &ops.string_list_ops {
            match &op.edit {
                StringListEdit::Insert { at, entries } if *at <= list.entries.len() => {
                    list.entries.splice(*at..*at, entries.clone());
                }
                StringListEdit::Remove { indices } => {
                    let selected: BTreeSet<_> = indices.iter().copied().collect();
                    list.entries = list
                        .entries
                        .into_iter()
                        .enumerate()
                        .filter(|(i, _)| !selected.contains(i))
                        .map(|(_, e)| e)
                        .collect();
                }
                _ => {}
            }
        }
        if same_slots(&before, &list.entries) {
            Ok(())
        } else {
            Err(EXPERT_REQUIRED.to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::loose_fixture::{LooseKit, set_reference};
    use crate::core::document::ops::{BlockOp, BlockOpKind, StringListOp};
    use blam_tags::classic::ClassicEngine;
    const GLOBAL: &str = "custom/prompts.hud_message_text";
    const MISSION: &str = "levels/a10/prompts.hud_message_text";

    fn tag(group: &str) -> TagFile {
        TagFile::new_classic(
            locate_definitions_root().join(format!("haloce_mcc/{group}.json")),
            ClassicEngine::HaloCe,
        )
        .unwrap()
    }
    fn entries() -> Vec<StringEntry> {
        ["pickup", "swap_weapon", "enter_vehicle"]
            .into_iter()
            .map(|id| StringEntry {
                id: Some(id.into()),
                texts: vec![Some(format!("{id} %action %custom-1"))],
            })
            .collect()
    }
    fn fixture() -> (LooseKit, Baboon, String, String) {
        let kit = LooseKit::new("hud-index-safety", "haloce_mcc");
        for path in [GLOBAL, MISSION] {
            let mut hud = tag("hud_message_text");
            strings::apply_string_list_edits(
                &mut hud,
                Some(GameId::HaloCe),
                &[StringListEdit::Insert {
                    at: 0,
                    entries: entries(),
                }],
            )
            .unwrap();
            let file = kit.path(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            hud.write(file).unwrap();
        }
        let mut globals = tag("hud_globals");
        set_reference(
            &mut globals,
            "hud messages",
            strings::HUD_GROUP,
            "CUSTOM\\PROMPTS",
        );
        let file = kit.path("ui/custom.hud_globals");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        globals.write(file).unwrap();
        let mut scenario = tag("scenario");
        set_reference(
            &mut scenario,
            "hud messages",
            strings::HUD_GROUP,
            "levels\\a10\\prompts",
        );
        scenario.write(kit.path("levels/a10/a10.scenario")).unwrap();
        let mut app = Baboon::for_test();
        kit.install(&mut app);
        let global = kit.open(&mut app, GLOBAL);
        let mission = kit.open(&mut app, MISSION);
        (kit, app, global, mission)
    }
    fn rows(app: &Baboon, key: &str) -> Vec<StringEntry> {
        strings::read_string_list(&app.model.kits[0].parsed_tags[key].tag)
            .unwrap()
            .entries
    }
    fn reorder() -> DeferredOps {
        DeferredOps {
            block_ops: vec![BlockOp {
                path: "messages".into(),
                kind: BlockOpKind::Reorder {
                    order: vec![1, 0, 2],
                },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn hud_index_safety_resolves_references_not_filename_and_unsaved_changes() {
        let (kit, mut app, global, mission) = fixture();
        assert_eq!(app.hud_message_usage(0, &global), HudMessageUsage::Global);
        assert_eq!(app.hud_message_usage(0, &mission), HudMessageUsage::Mission);
        let hudg = kit.open(&mut app, "ui/custom.hud_globals");
        set_reference(
            &mut app.model.kits[0].parsed_tags.get_mut(&hudg).unwrap().tag,
            "hud messages",
            strings::HUD_GROUP,
            "levels\\a10\\prompts",
        );
        assert_eq!(
            app.hud_message_usage(0, &mission),
            HudMessageUsage::Global,
            "unsaved HUD globals override disk and take precedence over mission use"
        );
        assert_eq!(app.hud_message_usage(0, &global), HudMessageUsage::Unknown);
    }
    #[test]
    fn hud_index_safety_blocks_reorder_without_history_and_allows_text_and_expert() {
        let (_kit, mut app, global, mission) = fixture();
        let before = rows(&app, &global);
        let stamp = app.model.kits[0].parsed_tags[&global].content_stamp();
        assert!(
            app.apply_doc_ops(0, &global, "Reorder", reorder(), UndoStep::Own)
                .is_none()
        );
        assert_eq!(rows(&app, &global), before);
        assert_eq!(
            app.model.kits[0].parsed_tags[&global].content_stamp(),
            stamp
        );
        assert!(app.model.status.contains("Expert Mode"));
        let ops = DeferredOps {
            string_list_ops: vec![StringListOp {
                game: Some(GameId::HaloCe),
                edit: StringListEdit::SetText {
                    index: 1,
                    language: 0,
                    text: "Edited %jump".into(),
                },
            }],
            ..Default::default()
        };
        assert!(
            app.apply_doc_ops(0, &global, "Text", ops, UndoStep::Own)
                .is_some()
        );
        assert_eq!(
            rows(&app, &global)[1].texts[0].as_deref(),
            Some("Edited %jump")
        );
        assert!(
            app.apply_doc_ops(0, &mission, "Reorder", reorder(), UndoStep::Own)
                .is_some()
        );
        assert_eq!(rows(&app, &mission)[0].id.as_deref(), Some("swap_weapon"));
        app.model.prefs.expert_mode = true;
        assert!(
            app.apply_doc_ops(0, &global, "Reorder", reorder(), UndoStep::Own)
                .is_some()
        );
        assert_eq!(rows(&app, &global)[0].id.as_deref(), Some("swap_weapon"));
        assert!(HudMessageUsage::Global.warning().contains("fixed indexes"));
    }
    #[test]
    fn hud_index_safety_guards_replace_import_but_allows_same_slots() {
        use crate::app::export::string_transfer::{
            ImportMode, StringDestination, StringFormat, StringTransferOptions,
        };
        let (kit, mut app, global, _) = fixture();
        let file = kit.base.join("import.csv");
        let options = StringTransferOptions {
            format: StringFormat::Csv,
            mode: Some(ImportMode::Replace),
            destination: StringDestination::Custom,
        };
        let before = rows(&app, &global);
        std::fs::write(
            &file,
            "Name,Text\nswap_weapon,Swap %action\npickup,Pickup\nenter_vehicle,Enter\n",
        )
        .unwrap();
        app.finish_string_transfer_options(
            app.model.kit_stamp(),
            &global,
            true,
            Some(0),
            &file,
            options,
        );
        assert_eq!(rows(&app, &global), before);
        assert!(
            app.model.status.contains("Expert Mode"),
            "{}",
            app.model.status
        );
        std::fs::write(
            &file,
            "Name,Text\npickup,Pickup\nswap_weapon,Swap %action\nenter_vehicle,Enter\n",
        )
        .unwrap();
        app.finish_string_transfer_options(
            app.model.kit_stamp(),
            &global,
            true,
            Some(0),
            &file,
            options,
        );
        assert_eq!(
            rows(&app, &global)[1].texts[0].as_deref(),
            Some("Swap %action"),
            "{}",
            app.model.status
        );
    }
    #[test]
    fn hud_index_safety_rejects_clipboard_source_removal_atomically_after_expert_off() {
        use super::super::string_row_clipboard::{
            self as clipboard, PasteMode, RowAction, RowCommand, RowDocument, RowRequest,
        };
        let (_kit, mut app, global, mission) = fixture();
        let ctx = egui::Context::default();
        let command = |app: &Baboon, key: &str, action| RowCommand {
            document: RowDocument {
                kit: app.model.kits[0].id,
                key: key.into(),
                id: app.model.kits[0].parsed_tags[key].id,
            },
            revision: None,
            request: RowRequest {
                action,
                selected: [0].into_iter().collect(),
                language: 0,
                scope: "test".into(),
            },
            confirmed: false,
            cut_token: None,
        };
        app.apply_string_row_command(command(&app, &global, RowAction::Copy { cut: true }), &ctx);
        assert!(clipboard::cut(&ctx).is_none());
        assert!(app.model.status.contains("Expert Mode"));
        app.model.prefs.expert_mode = true;
        app.apply_string_row_command(command(&app, &global, RowAction::Copy { cut: true }), &ctx);
        let data = clipboard::active(&ctx).unwrap().data.clone();
        app.model.prefs.expert_mode = false;
        let a = rows(&app, &global);
        let b = rows(&app, &mission);
        app.apply_string_row_command(
            command(
                &app,
                &mission,
                RowAction::Paste {
                    mode: PasteMode::Below,
                    data,
                },
            ),
            &ctx,
        );
        assert_eq!(rows(&app, &global), a);
        assert_eq!(rows(&app, &mission), b);
        assert!(app.model.status.contains("Expert Mode"));
    }
}
