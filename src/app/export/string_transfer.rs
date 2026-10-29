//! Spreadsheet/text codecs and one import policy shared by both formats.
use super::*;
use super::string_lists::{self, StringImportPlan, StringStoragePresentation};
use blam_tags::string_list::{self as strings, Storage, StringEntry, StringList, StringListEdit, TextSyntax};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum ImportMode { Amend, Replace }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum StringFormat { Text, Csv }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum StringMethod { Baboon, Tool }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum StringDestination { Kit, Custom }
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct StringTransferOptions {
    pub format: StringFormat,
    pub mode: Option<ImportMode>,
    pub destination: StringDestination,
}

struct ImportRow { id: Option<String>, index: Option<usize>, texts: Vec<Option<String>> }

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\r', '\n']) { format!("\"{}\"", text.replace('"', "\"\"")) }
    else { text.to_owned() }
}

/// Strict CSV reader: quoted commas, doubled quotes and multiline fields.
fn csv_rows(text: &str) -> Result<Vec<Vec<String>>, String> {
    delimited_rows(text, ',')
}

pub(in crate::app) fn delimited_rows(text: &str, delimiter: char) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut closed = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') { chars.next(); field.push('"'); }
                else { quoted = false; closed = true; }
            } else { field.push(c); }
        } else {
            match c {
                '"' if field.is_empty() && !closed => quoted = true,
                '"' => return Err("Unexpected quote in CSV field".to_owned()),
                c if c == delimiter => { row.push(std::mem::take(&mut field)); closed = false; }
                '\r' | '\n' => {
                    if c == '\r' && chars.peek() == Some(&'\n') { chars.next(); }
                    row.push(std::mem::take(&mut field));
                    if delimiter == '\t' || closed || row.len() > 1 || row.iter().any(|value| !value.is_empty()) { rows.push(std::mem::take(&mut row)); }
                    else { row.clear(); }
                    closed = false;
                }
                _ if closed => return Err("Unexpected text after a closing CSV quote".to_owned()),
                _ => field.push(c),
            }
        }
    }
    if quoted { return Err("CSV ends inside a quoted field".to_owned()); }
    if closed || !field.is_empty() || !row.is_empty() {
        row.push(field);
        if closed || row.len() > 1 || row.iter().any(|value| !value.is_empty()) { rows.push(row); }
    }
    Ok(rows)
}

pub(in crate::app) fn export_csv(list: &StringList, language: Option<usize>, syntax: &TextSyntax) -> Result<Vec<u8>, String> {
    let languages: Vec<_> = language.map_or_else(|| (0..list.languages.len()).collect(), |index| vec![index]);
    if languages.iter().any(|&index| index >= list.languages.len()) { return Err("Language no longer exists".to_owned()); }
    let mut header = vec![if list.storage == Storage::Classic { "Index".to_owned() } else { list.storage.identity_label().to_owned() }];
    header.extend(languages.iter().map(|&index| if list.storage != Storage::Multilingual { list.storage.text_label().to_owned() } else { format!("{} Text", list.languages[index].name) }));
    let mut text = format!("\u{feff}{}\r\n", header.iter().map(|value| csv_field(value)).collect::<Vec<_>>().join(","));
    let mut ids = BTreeSet::new();
    for (index, entry) in list.entries.iter().enumerate() {
        let identity = if list.storage == Storage::Classic { index.to_string() } else {
            let id = entry.id.clone().unwrap_or_default();
            if id.trim().is_empty() || !ids.insert(id.clone()) { return Err("CSV export requires unique, nonempty String IDs".to_owned()); }
            id
        };
        let mut row = vec![identity];
        row.extend(languages.iter().map(|&language| strings::format_shown(entry.texts[language].as_deref().unwrap_or_default(), syntax)));
        text.push_str(&row.iter().map(|value| csv_field(value)).collect::<Vec<_>>().join(","));
        text.push_str("\r\n");
    }
    Ok(text.into_bytes())
}

pub(in crate::app) fn plan_csv_import(list: &StringList, bytes: &[u8], mode: ImportMode, syntax: &TextSyntax) -> Result<StringImportPlan, String> {
    let text = string_lists::decode_text(bytes)?;
    let mut records = csv_rows(&text)?.into_iter();
    let header = records.next().ok_or("CSV has no header")?;
    let mut names = BTreeSet::new();
    let mut id_column = None;
    let mut index_column = None;
    let mut columns = Vec::new();
    for (column, name) in header.iter().enumerate() {
        let normalized = name.trim().to_lowercase();
        if !names.insert(normalized.clone()) { return Err(format!("Duplicate CSV column: {name}")); }
        match normalized.as_str() {
            "name" | "message name" | "string id" | "string_id" => { if id_column.replace(column).is_some() { return Err("Multiple String ID columns".to_owned()); } }
            "index" => index_column = Some(column),
            _ => {
                let language = if list.storage != Storage::Multilingual && matches!(normalized.as_str(), "string text" | "text") { Some(0) }
                else { list.languages.iter().position(|language| {
                    normalized == language.name.to_lowercase() || normalized == format!("{} text", language.name.to_lowercase())
                }) };
                let language = language.ok_or_else(|| format!("Unknown CSV column: {name}"))?;
                if columns.iter().any(|&(_, index)| index == language) { return Err(format!("Multiple columns for {}", list.languages[language].name)); }
                columns.push((column, language));
            }
        }
    }
    if list.storage.has_ids() && id_column.is_none() { return Err("CSV requires a String ID column".to_owned()); }
    if columns.is_empty() { return Err("CSV has no language text columns".to_owned()); }
    let mut rows = Vec::new();
    for (number, record) in records.enumerate() {
        if record.len() != header.len() { return Err(format!("CSV row {} has {} fields; expected {}", number + 2, record.len(), header.len())); }
        let index = index_column.filter(|&column| list.storage == Storage::Classic && mode == ImportMode::Amend && !record[column].trim().is_empty())
            .map(|column| record[column].trim().parse::<usize>().map_err(|_| format!("Invalid Index on CSV row {}", number + 2))).transpose()?;
        let mut texts = vec![None; list.languages.len()];
        for &(column, language) in &columns {
            let parsed = strings::parse_shown(&record[column], syntax).map_err(|error| format!("CSV row {}, {} contains invalid glyph/escape notation at {error:?}", number + 2, list.languages[language].name))?;
            if parsed.encode_utf16().count() > list.max_text_units() { return Err(format!("CSV row {} exceeds this engine's string length limit", number + 2)); }
            texts[language] = (!parsed.is_empty()).then_some(parsed);
        }
        rows.push(ImportRow { id: id_column.map(|column| record[column].clone()), index, texts });
    }
    plan_rows(list, rows, columns.iter().map(|&(_, language)| language).collect(), mode, 1)
}

fn plan_rows(list: &StringList, rows: Vec<ImportRow>, mut languages: BTreeSet<usize>, mode: ImportMode, files: usize) -> Result<StringImportPlan, String> {
    let mut notes = Vec::new();
    languages.retain(|&language| {
        let used = rows.iter().any(|row| row.texts[language].is_some());
        if !used { notes.push(format!("{} ignored: column is empty", list.languages[language].name)); }
        used
    });
    let mut seen = BTreeSet::new();
    let mut existing = HashMap::new();
    if list.storage.has_ids() {
        for (index, entry) in list.entries.iter().enumerate() {
            let id = entry.id.as_deref().unwrap_or_default();
            if existing.insert(id.to_owned(), index).is_some() { return Err(format!("Duplicate String ID in tag: {id}")); }
        }
    }
    let mut after = if mode == ImportMode::Amend { list.entries.clone() } else { Vec::new() };
    let mut updated = 0;
    let mut added = 0;
    let mut cleared = 0;
    let mut reordered = 0;
    for row in rows {
        let old = if list.storage.has_ids() {
            let id = row.id.as_deref().unwrap_or_default();
            if list.storage == Storage::HudMessages { strings::validate_hud_name(id)?; }
            if id.trim().is_empty() || id.trim() != id || id.chars().any(char::is_control) { return Err("String IDs must be nonempty and cannot contain surrounding whitespace or control characters".to_owned()); }
            if !seen.insert(id.to_owned()) { return Err(format!("Duplicate String ID in file: {id}")); }
            existing.get(id).copied()
        } else if mode == ImportMode::Amend {
            if let Some(index) = row.index {
                if index >= list.entries.len() { return Err(format!("Index {index} does not exist; leave Index blank to append a string")); }
                if !seen.insert(index.to_string()) { return Err(format!("Duplicate Index in file: {index}")); }
            }
            row.index
        } else { (after.len() < list.entries.len()).then_some(after.len()) };
        let mut entry = old.map_or_else(|| StringEntry { id: row.id.clone(), texts: vec![None; list.languages.len()] }, |index| list.entries[index].clone());
        if list.storage == Storage::Classic { entry.id = None; }
        for &language in &languages {
            if row.texts[language].is_some() || mode == ImportMode::Replace {
                if entry.texts[language].is_some() && row.texts[language].is_none() { cleared += 1; }
                entry.texts[language] = row.texts[language].clone();
            }
        }
        if let Some(index) = old {
            if entry != list.entries[index] { updated += 1; }
            if mode == ImportMode::Amend { after[index] = entry; }
            else { if index != after.len() { reordered += 1; } after.push(entry); }
        } else { added += 1; after.push(entry); }
    }
    let removed = if mode == ImportMode::Replace {
        if list.storage == Storage::Classic { list.entries.len().saturating_sub(after.len()) }
        else { existing.keys().filter(|id| !seen.contains(*id)).count() }
    } else { 0 };
    let edits = if after == list.entries { Vec::new() } else {
        let mut edits = Vec::new();
        if !list.entries.is_empty() { edits.push(StringListEdit::Remove { indices: (0..list.entries.len()).collect() }); }
        if !after.is_empty() { edits.push(StringListEdit::Insert { at: 0, entries: after }); }
        edits
    };
    Ok(StringImportPlan { edits, updated, added, removed, files, cleared, reordered, notes })
}

/// Text files participate in the same Amend/Replace policy as CSV.
pub(in crate::app) fn plan_text_import(list: &StringList, paths: &[(usize, PathBuf)], mode: ImportMode, syntax: &TextSyntax) -> Result<StringImportPlan, String> {
    let mut rows: Vec<ImportRow> = Vec::new();
    let mut by_id = HashMap::new();
    let mut languages = BTreeSet::new();
    let mut files = 0;
    for (language, path) in paths {
        if paths.len() > 1 && !path.exists() { continue; }
        let text = string_lists::decode_text(&fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?)?;
        files += 1;
        languages.insert(*language);
        if list.storage == Storage::Classic {
            for (index, text) in strings::parse_ce_strings_file(&text).into_iter().enumerate() {
                if text.encode_utf16().count() > list.max_text_units() { return Err("An imported string exceeds this engine's length limit".to_owned()); }
                rows.push(ImportRow { id: None, index: Some(index), texts: vec![(!text.is_empty()).then_some(text)] });
            }
        } else {
            let mut seen = BTreeSet::new();
            let parsed = if list.storage == Storage::HudMessages { strings::parse_hud_messages_file(&text, syntax)? } else { strings::parse_strings_file(&text, syntax)? };
            for (id, text) in parsed {
                if !seen.insert(id.clone()) { return Err(format!("Duplicate String ID in {}: {id}", path.display())); }
                if text.encode_utf16().count() > list.max_text_units() { return Err("An imported string exceeds this engine's length limit".to_owned()); }
                let index = *by_id.entry(id.clone()).or_insert_with(|| {
                    rows.push(ImportRow { id: Some(id), index: None, texts: vec![None; list.languages.len()] });
                    rows.len() - 1
                });
                rows[index].texts[*language] = (!text.is_empty()).then_some(text);
            }
        }
    }
    if files == 0 { return Err("No matching strings files found".to_owned()); }
    // Classic text has no identity column: Amend appends, like CSV without Index.
    if list.storage == Storage::Classic { for row in &mut rows { row.index = None; } }
    plan_rows(list, rows, languages, mode, files)
}

pub(super) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(|error| error.to_string())?; }
    let mut output = atomic_write_file::AtomicWriteFile::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    output.write_all(bytes).and_then(|_| output.commit()).map_err(|error| format!("{}: {error}", path.display()))
}

/// Canonical source paths, preserving the tag's relative tree.
pub(in crate::app) fn kit_text_paths(layout: &crate::core::source::KitLayout, game: Option<GameId>, relative: &Path, list: &StringList, language: Option<usize>) -> Result<Vec<(usize, PathBuf)>, String> {
    let mut paths = Vec::new();
    for index in language.map_or_else(|| (0..list.languages.len()).collect(), |index| vec![index]) {
        let name = list.languages.get(index).ok_or("Language no longer exists")?.name.to_lowercase();
        let suffix = match name.as_str() {
            "english" => None,
            "japanese" => Some("jpn"), "german" => Some("de"), "french" => Some("fr"),
            "spanish" => Some("sp"), "italian" => Some("it"), "korean" => Some("kor"),
            "chinese" | "traditional chinese" | "chinese (traditional)" => Some("cht"),
            "simplified chinese" | "chinese (simplified)" => Some("chs"),
            "portuguese" => Some("pt"), "polish" => Some("pl"), "russian" => Some("ru"),
            "mexican" | "spanish (mexican)" | "mexican spanish" | "spanish (latin america)" => Some("mx"),
            _ if list.storage != Storage::Multilingual => None,
            _ => return Err(format!("No verified data-folder mapping for {}", list.languages[index].name)),
        };
        let directory = if matches!(game, Some(GameId::Halo3 | GameId::Halo3Odst)) && suffix.is_some() {
            let long = match suffix {
                Some("mx") => "mexican", Some("cht") => "chinese", Some("chs") => "chinese_simplified", _ => &name,
            };
            layout.data_for_language(Some(long))
        } else { layout.data_for_language(suffix) };
        paths.push((index, directory.join(relative).with_extension(list.storage.text_extension())));
    }
    Ok(paths)
}

pub(in crate::app) fn transfer_paths(list: &StringList, language: Option<usize>, stem: &str, relative: &Path, path: &Path, layout: Option<&crate::core::source::KitLayout>, game: Option<GameId>, options: StringTransferOptions) -> Result<Vec<(usize, PathBuf)>, String> {
    if relative.is_absolute() || relative.components().any(|part| !matches!(part, std::path::Component::Normal(_))) {
        return Err("Invalid tag-relative source path".to_owned());
    }
    if options.format == StringFormat::Csv {
        let path = if options.destination == StringDestination::Kit { layout.ok_or("No editing kit data folder")?.data.join(relative).with_extension("csv") } else { path.to_owned() };
        return Ok(vec![(0, path)]);
    }
    if options.destination == StringDestination::Kit { return kit_text_paths(layout.ok_or("No editing kit data folder")?, game, relative, list, language); }
    let mut paths = Vec::new();
    for index in language.map_or_else(|| (0..list.languages.len()).collect(), |index| vec![index]) {
        if index >= list.languages.len() { return Err("Language no longer exists".to_owned()); }
        paths.push((index, if language.is_some() { path.to_owned() } else { path.join(string_lists::string_file_name(stem, list, index)) }));
    }
    Ok(paths)
}

pub(in crate::app) fn transfer_import_plan(list: &StringList, paths: &[(usize, PathBuf)], options: StringTransferOptions, syntax: &TextSyntax) -> Result<StringImportPlan, String> {
    let mode = options.mode.ok_or("Choose Amend or Replace before importing")?;
    match options.format {
        StringFormat::Csv => plan_csv_import(list, &fs::read(&paths[0].1).map_err(|error| error.to_string())?, mode, syntax),
        StringFormat::Text => plan_text_import(list, paths, mode, syntax),
    }
}

/// Only commands documented for the engine are offered. Shell metacharacters
/// are refused because the existing terminal runner accepts a command line.
pub(in crate::app) fn tool_string_command(game: Option<GameId>, import: bool, relative: &Path) -> Result<String, String> {
    let path = relative.to_string_lossy().replace('/', "\\");
    if path.chars().any(|c| c.is_control() || "\"&|<>^%!`$".contains(c)) || relative.is_absolute() || relative.components().any(|part| !matches!(part, std::path::Component::Normal(_))) {
        return Err("This tag path cannot be safely passed to tool.exe".to_owned());
    }
    let parent = path.rsplit_once('\\').map_or("", |(parent, _)| parent);
    let (verb, argument) = match (game, import) {
        (Some(GameId::Halo2), false) => ("extract-unicode-strings", path.as_str()),
        (Some(GameId::Halo2), true) => ("new-strings", parent),
        (Some(GameId::HaloCe), true) => ("unicode-strings", parent),
        _ => return Err("No verified Unicode-string tool command is available for this engine and operation. Use Baboon instead.".to_owned()),
    };
    Ok(format!("tool {verb} \"{argument}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(game: GameId) -> (TagFile, TextSyntax) {
        let root = crate::core::bundled::locate_definitions_root();
        let group = if game == GameId::HaloCe { "unicode_string_list" } else { "multilingual_unicode_string_list" };
        let mut tag = TagFile::new(root.join(game.as_str()).join(format!("{group}.json"))).unwrap();
        let list = strings::read_string_list(&tag).unwrap();
        let entries = (0..3).map(|row| StringEntry {
            id: (game != GameId::HaloCe).then(|| format!("s{row}")),
            texts: (0..list.languages.len()).map(|language| {
                (language == 0 || row % 2 == 0).then(|| format!("{row}:{language} 日本語 한글 中文 , \"quoted\"\r\nnext \u{e156}"))
            }).collect(),
        }).collect();
        strings::apply_string_list_edits(&mut tag, Some(game), &[StringListEdit::Insert { at: 0, entries }]).unwrap();
        (tag, TextSyntax::load(root.join(game.as_str())).unwrap())
    }

    fn imported(mut tag: TagFile, game: GameId, csv: &str, mode: ImportMode) -> (StringList, StringImportPlan) {
        let syntax = crate::app::editor::string_list::text_syntax(None, Some(game));
        let before = strings::read_string_list(&tag).unwrap();
        let plan = plan_csv_import(&before, csv.as_bytes(), mode, &syntax).unwrap();
        strings::apply_string_list_edits(&mut tag, Some(game), &plan.edits).unwrap();
        let bytes = tag.write_to_bytes().unwrap();
        let reopened = TagFile::read_from_bytes(&bytes).unwrap();
        let list = strings::read_string_list(&reopened).unwrap();
        assert_eq!(list.damaged_offsets, 0);
        (list, plan)
    }

    #[test]
    fn csv_round_trips_all_engines_languages_quotes_and_unicode() {
        for game in GameId::ALL {
            let (tag, syntax) = fixture(game);
            let list = strings::read_string_list(&tag).unwrap();
            for language in std::iter::once(None).chain((0..list.languages.len()).map(Some)) {
                let bytes = export_csv(&list, language, &syntax).unwrap();
                assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
                for mode in [ImportMode::Amend, ImportMode::Replace] {
                    let plan = plan_csv_import(&list, &bytes, mode, &syntax).unwrap();
                    assert!(plan.edits.is_empty(), "{game:?} {language:?} {mode:?}");
                }
            }
        }
    }

    #[test]
    fn csv_amend_keeps_order_and_blanks_replace_deletes_reorders_and_preserves_other_languages() {
        let (tag, _) = fixture(GameId::Halo3);
        let before = strings::read_string_list(&tag).unwrap();
        let csv = "String ID,English Text,Japanese Text\r\ns2,Changed,\r\ns0,,\r\nnew_id,新しい\u{e156},\r\n";
        let (amend, plan) = imported(tag, GameId::Halo3, csv, ImportMode::Amend);
        assert_eq!(amend.entries.iter().map(|entry| entry.id.as_deref().unwrap()).collect::<Vec<_>>(), ["s0", "s1", "s2", "new_id"]);
        assert_eq!(amend.entries[0], before.entries[0]);
        assert_eq!(amend.entries[1], before.entries[1]);
        assert_eq!(amend.entries[2].texts[0].as_deref(), Some("Changed"));
        assert_eq!(amend.entries[2].texts[1], before.entries[2].texts[1]);
        assert_eq!((plan.added, plan.removed, plan.cleared), (1, 0, 0));
        assert!(plan.notes.iter().any(|note| note.contains("Japanese ignored")));
        let (tag, _) = fixture(GameId::Halo3);
        let (replace, plan) = imported(tag, GameId::Halo3, csv, ImportMode::Replace);
        assert_eq!(replace.entries.iter().map(|entry| entry.id.as_deref().unwrap()).collect::<Vec<_>>(), ["s2", "s0", "new_id"]);
        assert_eq!(replace.entries[1].texts[0], None);
        assert_eq!(replace.entries[0].texts[1..], before.entries[2].texts[1..]);
        assert_eq!(replace.entries[1].texts[1..], before.entries[0].texts[1..]);
        assert_eq!((plan.added, plan.removed, plan.cleared), (1, 1, 1));
        assert!(plan.reordered > 0);
        let (tag, _) = fixture(GameId::Halo3);
        let (replace, plan) = imported(tag, GameId::Halo3, "String ID,Japanese Text\ns0,\ns2,翻訳\n", ImportMode::Replace);
        assert_eq!(replace.entries[0].texts[0], before.entries[0].texts[0]);
        assert_eq!(replace.entries[0].texts[1], None);
        assert_eq!(plan.cleared, 1);
    }

    #[test]
    fn hud_csv_and_hmt_share_named_amend_replace_and_lossless_exports() {
        let root=crate::core::bundled::locate_definitions_root();
        let mut tag=TagFile::new(root.join("haloce_mcc/hud_message_text.json")).unwrap();
        let entries=vec![StringEntry {id:Some("one".into()),texts:vec![Some("First %{18}".into())]},StringEntry {id:Some("two".into()),texts:vec![Some("Second %{32}".into())]}];
        strings::apply_string_list_edits(&mut tag,Some(GameId::HaloCe),&[StringListEdit::Insert {at:0,entries}]).unwrap();
        let list=strings::read_string_list(&tag).unwrap();
        let syntax=list.storage.text_syntax(TextSyntax::load(root.join("haloce_mcc")).unwrap());
        let csv=export_csv(&list,None,&syntax).unwrap();
        assert!(String::from_utf8(csv.clone()).unwrap().starts_with("\u{feff}Name,Text\r\n"));
        assert!(plan_csv_import(&list,&csv,ImportMode::Replace,&syntax).unwrap().edits.is_empty());
        let amend=plan_csv_import(&list,b"Name,Text\ntwo,Changed %jump\nthree,New\n",ImportMode::Amend,&syntax).unwrap();
        strings::apply_string_list_edits(&mut tag,Some(GameId::HaloCe),&amend.edits).unwrap();
        let amended=strings::read_string_list(&tag).unwrap();
        assert_eq!(amended.entries[0],list.entries[0]); assert_eq!(amended.entries.len(),3);
        assert_eq!(amended.entries[1].texts[0].as_deref(),Some("Changed %{22}"));
        let native=super::string_lists::export_text(&amended,0,&syntax).unwrap();
        let folder=std::env::temp_dir().join(format!("baboon-hud-{}",uuid::Uuid::new_v4())); fs::create_dir_all(&folder).unwrap();
        let source=folder.join("messages.hmt"); fs::write(&source,super::string_lists::encode_text(&native)).unwrap();
        assert!(plan_text_import(&amended,&[(0,source.clone())],ImportMode::Replace,&syntax).unwrap().edits.is_empty());
        let replaced=plan_csv_import(&amended,b"Name,Text\nthree,%zoom\none,\n",ImportMode::Replace,&syntax).unwrap();
        assert_eq!((replaced.removed,replaced.cleared),(1,1));
        strings::apply_string_list_edits(&mut tag,Some(GameId::HaloCe),&replaced.edits).unwrap();
        let replaced=strings::read_string_list(&tag).unwrap(); assert_eq!(replaced.entries[0].id.as_deref(),Some("three")); assert_eq!(replaced.entries[1].texts[0],None);
        assert!(plan_csv_import(&replaced,b"Name,Text\nthree,%unknown",ImportMode::Replace,&syntax).is_err());
        let layout=crate::core::source::KitLayout::from_tags_folder(Path::new("kit/tags")).unwrap();
        assert_eq!(kit_text_paths(&layout,Some(GameId::HaloCe),Path::new("ui/hud/messages"),&list,Some(0)).unwrap()[0].1,PathBuf::from("kit/data/ui/hud/messages.hmt"));
        fs::remove_file(source).unwrap(); fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn classic_csv_indices_and_indexless_modes() {
        let (tag, _) = fixture(GameId::HaloCe);
        let before = strings::read_string_list(&tag).unwrap();
        let (amend, _) = imported(tag, GameId::HaloCe, "Index,String Text\n1,Changed\n,Added\n", ImportMode::Amend);
        assert_eq!(amend.entries.len(), 4);
        assert_eq!(amend.entries[0], before.entries[0]);
        assert_eq!(amend.entries[1].texts[0].as_deref(), Some("Changed"));
        assert_eq!(amend.entries[3].texts[0].as_deref(), Some("Added"));
        let (tag, _) = fixture(GameId::HaloCe);
        let (amend, _) = imported(tag, GameId::HaloCe, "String Text\nOne\nTwo\n", ImportMode::Amend);
        assert_eq!(amend.entries.len(), 5);
        assert_eq!(amend.entries[..3], before.entries);
        for csv in ["String Text\nOne\nTwo\n", "Index,String Text\n999,One\n0,Two\n"] {
            let (tag, _) = fixture(GameId::HaloCe);
            let (replace, _) = imported(tag, GameId::HaloCe, csv, ImportMode::Replace);
            assert_eq!(replace.entries.iter().map(|entry| entry.texts[0].as_deref().unwrap()).collect::<Vec<_>>(), ["One", "Two"]);
        }
    }

    #[test]
    fn csv_parser_handles_spreadsheet_quotes_multiline_and_rejects_invalid_input() {
        assert_eq!(csv_rows("A,B\r\n\"comma, quote \"\" and\r\nline\",value\r\n").unwrap()[1], ["comma, quote \" and\r\nline", "value"]);
        assert_eq!(csv_rows("A\n\"\"\n").unwrap().len(), 2);
        for text in ["A\n\"unfinished", "A\n\"closed\"junk", "A\nnot\"quoted"] { assert!(csv_rows(text).is_err()); }
        let (tag, syntax) = fixture(GameId::Halo3);
        let list = strings::read_string_list(&tag).unwrap();
        for csv in [
            "String ID,English Text\ns0,One\ns0,Two", "String ID,English Text\n,Text",
            "String ID,English Text\ns0,Too,many", "String ID,Unknown Text\ns0,Text",
            "String ID,English Text\ns0,&unknown_glyph", "String ID,English Text,english text\ns0,One,Two",
        ] { assert!(plan_csv_import(&list, csv.as_bytes(), ImportMode::Replace, &syntax).is_err(), "{csv}"); }
    }

    #[test]
    fn tool_commands_are_engine_specific_and_reject_shell_paths() {
        let relative = Path::new("ui/hud/messages");
        assert_eq!(tool_string_command(Some(GameId::Halo2), false, relative).unwrap(), "tool extract-unicode-strings \"ui\\hud\\messages\"");
        assert_eq!(tool_string_command(Some(GameId::Halo2), true, relative).unwrap(), "tool new-strings \"ui\\hud\"");
        assert_eq!(tool_string_command(Some(GameId::HaloCe), true, relative).unwrap(), "tool unicode-strings \"ui\\hud\"");
        assert!(tool_string_command(Some(GameId::Halo3), true, relative).is_err());
        for path in ["../outside", "ui/a&b", "ui/quoted\"", "ui/%temp%"] { assert!(tool_string_command(Some(GameId::Halo2), true, Path::new(path)).is_err()); }
    }

    #[test]
    fn kit_paths_use_halo2_language_folders_and_preserve_the_tag_tree() {
        let (tag, _) = fixture(GameId::Halo2);
        let list = strings::read_string_list(&tag).unwrap();
        let layout = crate::core::source::KitLayout::from_tags_folder(Path::new("kit/tags")).unwrap();
        let paths = kit_text_paths(&layout, Some(GameId::Halo2), Path::new("ui/hud/messages"), &list, None).unwrap();
        assert!(paths.iter().any(|(_, path)| path == &PathBuf::from("kit/data/ui/hud/messages.txt")));
        assert!(paths.iter().any(|(_, path)| path == &PathBuf::from("kit/data_fr/ui/hud/messages.txt")));
        assert!(paths.iter().any(|(_, path)| path == &PathBuf::from("kit/data_jpn/ui/hud/messages.txt")));
    }
}
