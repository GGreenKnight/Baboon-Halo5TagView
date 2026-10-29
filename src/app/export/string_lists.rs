//! Editable string-list text files and the plans used to import them atomically.

#[cfg(test)]
use super::*;
use blam_tags::string_list::{self as strings, Storage, StringList, StringListEdit, TextSyntax};
#[cfg(test)]
use blam_tags::string_list::StringEntry;
use std::collections::BTreeSet;
#[cfg(test)]
use std::collections::HashMap;

/// Labels and file choices are application presentation, not engine storage rules.
pub(in crate::app) trait StringStoragePresentation {
    fn identity_label(self) -> &'static str;
    fn text_label(self) -> &'static str;
    fn text_extension(self) -> &'static str;
}

impl StringStoragePresentation for Storage {
    fn identity_label(self) -> &'static str {if self==Self::HudMessages {"Name"}else{"String ID"}}
    fn text_label(self) -> &'static str {if self==Self::HudMessages {"Text"}else{"String Text"}}
    fn text_extension(self) -> &'static str {if self==Self::HudMessages {"hmt"}else{"txt"}}
}

pub(in crate::app) fn string_file_name(stem: &str, list: &StringList, language: usize) -> String {
    if list.storage == Storage::HudMessages { return format!("{stem}.hmt"); }
    if list.storage == Storage::Classic { return format!("{stem}.txt"); }
    let language = list.languages[language].name.to_ascii_lowercase().replace(' ', "_");
    format!("{stem}.{language}.txt")
}

/// Tool-readable files use UTF-16 LE with a BOM; ordinary editors also save UTF-8.
pub(super) fn encode_text(text: &str) -> Vec<u8> {
    [0xff, 0xfe].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect()
}

pub(super) fn decode_text(bytes: &[u8]) -> Result<String, String> {
    let utf16 = if let Some(body) = bytes.strip_prefix(&[0xff, 0xfe]) {
        Some((body, true))
    } else { bytes.strip_prefix(&[0xfe, 0xff]).map(|body| (body, false)) };
    if let Some((body, little)) = utf16 {
        if body.len() % 2 != 0 { return Err("UTF-16 file ends with an incomplete character".to_owned()); }
        let units: Vec<_> = body.chunks_exact(2).map(|pair| {
            if little { u16::from_le_bytes([pair[0], pair[1]]) } else { u16::from_be_bytes([pair[0], pair[1]]) }
        }).collect();
        let text = String::from_utf16(&units).map_err(|error| format!("Invalid UTF-16: {error}"))?;
        if text.contains('\0') { return Err("Strings files cannot contain NUL characters".to_owned()); }
        Ok(text)
    } else {
        let text = std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
            .map_err(|_| "Save the file as UTF-8 or UTF-16 with a byte-order mark".to_owned())?;
        if text.contains('\0') { return Err("Save UTF-16 files with a byte-order mark".to_owned()); }
        Ok(text.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::document::apply::{DeferredOps, apply_deferred_ops};
    use crate::core::document::ops::StringListOp;

    fn scratch() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("test-string-files-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn fixture(game: GameId) -> (TagFile, TextSyntax) {
        let group = if game == GameId::HaloCe { "unicode_string_list" } else { "multilingual_unicode_string_list" };
        let root = crate::core::bundled::locate_definitions_root();
        let mut tag = TagFile::new(root.join(game.as_str()).join(format!("{group}.json"))).unwrap();
        let list = strings::read_string_list(&tag).unwrap();
        let entries = (0..3).map(|index| StringEntry {
            id: (game != GameId::HaloCe).then(|| format!("s{index}")),
            texts: (0..list.languages.len()).map(|language| {
                (language == 0 || (index + language) % 3 != 0).then(|| format!("{index}:{language} 日本語 \u{E156} & \"quote\"\r\nnext"))
            }).collect(),
        }).collect();
        strings::apply_string_list_edits(&mut tag, Some(game), &[StringListEdit::Insert { at: 0, entries }]).unwrap();
        (tag, TextSyntax::load(root.join(game.as_str())).unwrap())
    }

    #[test]
    fn string_files_round_trip_every_language_in_every_engine() {
        let folder = scratch();
        for game in GameId::ALL {
            let (tag, syntax) = fixture(game);
            let list = strings::read_string_list(&tag).unwrap();
            let directory = folder.join(game.as_str());
            fs::create_dir_all(&directory).unwrap();
            assert_eq!(write_string_files(&list, None, "messages", &directory, &syntax).unwrap(), list.languages.len());
            for language in 0..list.languages.len() {
                let bytes = fs::read(directory.join(string_file_name("messages", &list, language))).unwrap();
                assert!(bytes.starts_with(&[0xff, 0xfe]));
            }
            let plan = plan_string_import(&list, None, "messages", &directory, &syntax).unwrap();
            assert!(plan.edits.is_empty(), "{game:?}: exported files must not change anything on import");
            assert_eq!(plan.files, list.languages.len());
        }
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn string_file_import_merges_ids_preserves_translations_saves_and_undoes() {
        let folder = scratch();
        for game in GameId::ALL.into_iter().filter(|game| *game != GameId::HaloCe) {
            let (tag, syntax) = fixture(game);
            let before_list = strings::read_string_list(&tag).unwrap();
            let path = folder.join(format!("{}.txt", game.as_str()));
            fs::write(&path, "[Strings]\ns0 = \"Edited 日本語\\r\\n&U+E156\"\nnew_string = \"New translation\"\n").unwrap();
            let plan = plan_string_import(&before_list, Some(1), "unused", &path, &syntax).unwrap();
            assert_eq!((plan.updated, plan.added, plan.removed), (1, 1, 0));
            let mut doc = TagDocument::clean(tag);
            let before = doc.tag.write_to_bytes().unwrap();
            apply_deferred_ops(&mut doc, DeferredOps {
                string_list_ops: plan.edits.into_iter().map(|edit| StringListOp { game: Some(game), edit }).collect(),
                ..Default::default()
            }, "Import strings");
            let bytes = doc.tag.write_to_bytes().unwrap();
            let reopened = TagFile::read_from_bytes(&bytes).unwrap();
            let after = strings::read_string_list(&reopened).unwrap();
            assert_eq!(after.entries.len(), 4);
            assert_eq!(after.entries[0].texts[1].as_deref(), Some("Edited 日本語\r\n\u{E156}"));
            assert_eq!(after.entries[0].texts[0], before_list.entries[0].texts[0]);
            assert_eq!(after.entries[1..3], before_list.entries[1..3]);
            assert_eq!(after.damaged_offsets, 0);
            for language in 0..after.languages.len() {
                if language != 1 { assert_eq!(after.entries[3].texts[language], None); }
            }
            doc.journal.end_edit_window();
            assert_eq!(*doc.journal.undo(&doc.tag).unwrap().0, before);
        }
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn classic_string_file_import_replaces_in_file_order() {
        let folder = scratch();
        let (mut tag, syntax) = fixture(GameId::HaloCe);
        let list = strings::read_string_list(&tag).unwrap();
        let path = folder.join("ce.txt");
        fs::write(&path, encode_text(&strings::ce_strings_file(["Second 日本語", "First |n line"])) ).unwrap();
        let plan = plan_string_import(&list, Some(0), "unused", &path, &syntax).unwrap();
        assert_eq!((plan.updated, plan.added, plan.removed), (2, 0, 1));
        strings::apply_string_list_edits(&mut tag, Some(GameId::HaloCe), &plan.edits).unwrap();
        let after = strings::read_string_list(&tag).unwrap();
        assert_eq!(after.entries.iter().map(|entry| entry.texts[0].as_deref().unwrap()).collect::<Vec<_>>(), vec!["Second 日本語", "First |n line"]);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn string_files_accept_utf8_and_both_utf16_orders_and_reject_bad_input() {
        let text = "日本語 🙂";
        assert_eq!(decode_text(text.as_bytes()).unwrap(), text);
        assert_eq!(decode_text(&encode_text(text)).unwrap(), text);
        let big: Vec<_> = [0xfe, 0xff].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode_text(&big).unwrap(), text);
        assert!(decode_text(&[0xff, 0xfe, 0]).is_err());
        assert!(decode_text(&[0xff, 0xfe, 0, 0xd8]).is_err());
        assert!(decode_text(b"s0\0").is_err());
        let folder = scratch();
        let (tag, syntax) = fixture(GameId::Halo3);
        let list = strings::read_string_list(&tag).unwrap();
        let file = folder.join("broken.txt");
        for contents in ["s0 = \"Valid\"\nthis line is broken", "s0 = \"First\"\ns0 = \"Duplicate\"", "s0 = \"&unknown_glyph\""] {
            fs::write(&file, contents).unwrap();
            assert!(plan_string_import(&list, Some(0), "unused", &file, &syntax).is_err());
        }
        // Missing languages are left alone; an invalid present language rejects the whole batch.
        fs::write(folder.join(string_file_name("messages", &list, 0)), "s0 = \"Valid\"").unwrap();
        let plan = plan_string_import(&list, None, "messages", &folder, &syntax).unwrap();
        assert_eq!(plan.files, 1);
        fs::write(folder.join(string_file_name("messages", &list, 1)), "malformed").unwrap();
        assert!(plan_string_import(&list, None, "messages", &folder, &syntax).is_err());
        fs::remove_dir_all(folder).unwrap();
    }
}

pub(super) fn export_text(list: &StringList, language: usize, syntax: &TextSyntax) -> Result<String, String> {
    if list.storage == Storage::HudMessages { return strings::hud_messages_file(&list.entries, syntax); }
    if language >= list.languages.len() { return Err("That language is no longer available".to_owned()); }
    if list.storage == Storage::Classic {
        if list.entries.iter().any(|entry| entry.texts[0].as_deref().is_some_and(|text| text.contains("###END-STRING###"))) {
            return Err("A string contains the file format's ###END-STRING### separator".to_owned());
        }
        return Ok(strings::ce_strings_file(list.entries.iter().map(|entry| entry.texts[0].as_deref().unwrap_or_default())));
    }
    let mut ids = BTreeSet::new();
    let mut text = "[Strings]\r\n".to_owned();
    for entry in &list.entries {
        let id = entry.id.as_deref().unwrap_or_default();
        if id.is_empty() || id.trim() != id || id.contains(['=', '\r', '\n']) {
            return Err(format!("String ID {id:?} cannot be represented in a strings text file"));
        }
        if !ids.insert(id) { return Err(format!("String ID {id:?} is duplicated; give each string a unique ID before exporting")); }
        text.push_str(&strings::strings_file_line(id, entry.texts[language].as_deref().unwrap_or_default(), syntax));
        text.push_str("\r\n");
    }
    Ok(text)
}

#[cfg(test)]
pub(in crate::app) fn write_string_files(list: &StringList, language: Option<usize>, stem: &str, path: &Path, syntax: &TextSyntax) -> Result<usize, String> {
    let languages: Vec<_> = language.map_or_else(|| (0..list.languages.len()).collect(), |language| vec![language]);
    // Validate every language before writing the first file.
    let files: Vec<_> = languages.into_iter().map(|index| {
        let text = export_text(list, index, syntax)?;
        let output = if language.is_some() { path.to_owned() } else { path.join(string_file_name(stem, list, index)) };
        Ok((output, encode_text(&text)))
    }).collect::<Result<_, String>>()?;
    for (output, bytes) in &files {
        use std::io::Write;
        let mut file = atomic_write_file::AtomicWriteFile::open(output).map_err(|error| format!("Could not open {}: {error}", output.display()))?;
        file.write_all(bytes).and_then(|_| file.commit()).map_err(|error| format!("Could not finish writing {}: {error}", output.display()))?;
    }
    Ok(files.len())
}

pub(in crate::app) struct StringImportPlan {
    pub(in crate::app) edits: Vec<StringListEdit>,
    pub(in crate::app) updated: usize,
    pub(in crate::app) added: usize,
    pub(in crate::app) removed: usize,
    pub(in crate::app) files: usize,
    pub(in crate::app) cleared: usize,
    pub(in crate::app) reordered: usize,
    pub(in crate::app) notes: Vec<String>,
}

#[cfg(test)]
pub(in crate::app) fn plan_string_import(list: &StringList, language: Option<usize>, stem: &str, path: &Path, syntax: &TextSyntax) -> Result<StringImportPlan, String> {
    let languages: Vec<_> = language.map_or_else(|| (0..list.languages.len()).collect(), |language| vec![language]);
    let mut files = Vec::new();
    for index in languages {
        if index >= list.languages.len() { return Err("That language is no longer available".to_owned()); }
        let input = if language.is_some() { path.to_owned() } else { path.join(string_file_name(stem, list, index)) };
        // All-languages import accepts a partial set, leaving missing languages untouched.
        if language.is_none() && !input.exists() { continue; }
        let bytes = fs::read(&input).map_err(|error| format!("{}: {error}", input.display()))?;
        let text = decode_text(&bytes).map_err(|error| format!("{}: {error}", input.display()))?;
        files.push((index, text));
    }
    if files.is_empty() { return Err(format!("No matching strings files found for {stem}")); }
    let mut plan = StringImportPlan { edits: Vec::new(), updated: 0, added: 0, removed: 0, files: files.len(), cleared: 0, reordered: 0, notes: Vec::new() };
    if list.storage == Storage::Classic {
        let entries: Vec<_> = strings::parse_ce_strings_file(&files[0].1).into_iter()
            .map(|text| StringEntry { id: None, texts: vec![Some(text)] }).collect();
        if entries.is_empty() && !list.entries.is_empty() { return Err("The file contains no strings".to_owned()); }
        if entries.iter().any(|entry| entry.texts[0].as_ref().unwrap().encode_utf16().count() > list.max_text_units()) {
            return Err("An imported string exceeds this engine's text length limit".to_owned());
        }
        if entries != list.entries {
            plan.updated = entries.len().min(list.entries.len());
            plan.added = entries.len().saturating_sub(list.entries.len());
            plan.removed = list.entries.len().saturating_sub(entries.len());
            if !list.entries.is_empty() { plan.edits.push(StringListEdit::Remove { indices: (0..list.entries.len()).collect() }); }
            if !entries.is_empty() { plan.edits.push(StringListEdit::Insert { at: 0, entries }); }
        }
        return Ok(plan);
    }
    let mut indices = HashMap::new();
    for (index, entry) in list.entries.iter().enumerate() {
        let id = entry.id.clone().unwrap_or_default();
        if indices.insert(id.clone(), index).is_some() { return Err(format!("String ID {id:?} is duplicated in the tag; make IDs unique before importing")); }
    }
    let mut entries = list.entries.clone();
    for (language, text) in files {
        let parsed = strings::parse_strings_file(&text, syntax)?;
        let mut seen = BTreeSet::new();
        for (id, text) in parsed {
            if !seen.insert(id.clone()) { return Err(format!("String ID {id:?} appears more than once in {}", list.languages[language].name)); }
            if text.encode_utf16().count() > list.max_text_units() { return Err(format!("String {id:?} exceeds this engine's text length limit")); }
            let index = *indices.entry(id.clone()).or_insert_with(|| {
                let index = entries.len();
                entries.push(StringEntry { id: Some(id), texts: vec![None; list.languages.len()] });
                index
            });
            entries[index].texts[language] = (!text.is_empty()).then_some(text);
        }
    }
    for (index, (before, after)) in list.entries.iter().zip(&entries).enumerate() {
        if before != after { plan.updated += 1; }
        for (language, (before, after)) in before.texts.iter().zip(&after.texts).enumerate() {
            if before != after { plan.edits.push(StringListEdit::SetText { index, language, text: after.clone().unwrap_or_default() }); }
        }
    }
    let added: Vec<_> = entries.into_iter().skip(list.entries.len()).collect();
    plan.added = added.len();
    if !added.is_empty() { plan.edits.push(StringListEdit::Insert { at: list.entries.len(), entries: added }); }
    Ok(plan)
}
