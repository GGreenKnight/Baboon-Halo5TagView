//! The Strings tab of a string list tag (`multilingual_unicode_string_list`,
//! and Halo CE's `unicode_string_list`): every string in a table, each text
//! editable as the game's tool writes it, with `&name` glyphs shown by name,
//! and strings added, inserted, duplicated, removed, copied and pasted.
//!
//! Foundation's Strings section only lists the text, for one language, with
//! glyphs drawn as boxes. What is read and written, and how text is shown, is
//! the engine's (`blam_tags::string_list`); this module is the table.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

use blam_tags::string_list::{
    self as strings, StringEntry, StringList, StringListEdit, Storage, TextSyntax, TokenKind,
};

use crate::core::document::ops::StringListOp;

use egui_extras::{Column, TableBuilder};

use super::*;

/// Whether the tag pane shows the strings or the plain field tree.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum StringListTab {
    #[default]
    Strings,
    Fields,
}

/// How many choices the completion list shows before it scrolls and gains
/// a filter box.
const COMPLETION_ROWS: usize = 12;
const ROW_HEIGHT: f32 = 24.0;
/// The gap the text leaves before a colour for its swatch.
const SWATCH_SPACE: f32 = 16.0;
const SWATCH_SIZE: f32 = 10.0;

/// What the Strings tab remembers between frames, per pane and tag.
#[derive(Clone, Default)]
struct StringListView {
    tab: StringListTab,
    language: usize,
    filter: String,
    /// Selected strings, by index into the list.
    selected: BTreeSet<usize>,
    /// Where a shift-click extends the selection from.
    anchor: Option<usize>,
    /// A removal waiting on the user to confirm it.
    confirm_remove: bool,
    /// Scroll this string into view on the next frame.
    scroll_to: Option<usize>,
    completion: CompletionView,
}

/// The completion list of the cell being typed in.
#[derive(Clone, Default)]
struct CompletionView {
    /// The draft key of the cell it was last shown for.
    cell: String,
    /// What it completes, while it is open.
    open: Option<strings::Completion>,
    /// The highlighted row of the filtered list.
    highlighted: usize,
    /// Scroll the highlighted row into view on the next frame.
    scroll: bool,
    /// What the list's filter box holds.
    filter: String,
    /// The filter box had focus at the end of the last frame.
    filter_focused: bool,
    /// The pointer was over the list last frame: a click there takes focus
    /// from the cell, and must not commit what it holds.
    hovered: bool,
    /// The `&` or `#` whose list Escape closed; it stays closed until the
    /// cursor leaves that name.
    dismissed_at: Option<usize>,
}

impl CompletionView {
    fn close(&mut self) {
        self.open = None;
        self.filter.clear();
        self.filter_focused = false;
        self.hovered = false;
    }
}

/// The tag's strings, read once per change to the tag.
struct ListCache {
    revision: (u64, u64, u64, u64),
    list: Arc<StringList>,
    /// The list's strings for the current language and filter.
    filter_key: (usize, String),
    visible: Arc<Vec<usize>>,
}

/// The game's string text syntax, read once per definitions folder and game
/// from its `_meta.json`. Sources with no definitions folder of their own
/// read the bundled one; a game that can't be read gets only `&&` and
/// `&U+XXXX`.
pub(in crate::app) fn text_syntax(definitions_root: Option<&Path>, game: Option<GameId>) -> TextSyntax {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, GameId), TextSyntax>>> = OnceLock::new();
    let Some(game) = game else {
        return TextSyntax::unknown();
    };
    let root = definitions_root.map_or_else(crate::core::bundled::locate_definitions_root, Path::to_path_buf);
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry((root.clone(), game))
        .or_insert_with(|| {
            TextSyntax::load(root.join(game.as_str())).unwrap_or_else(|error| {
                eprintln!("string text syntax for {}: {error}", game.as_str());
                TextSyntax::unknown()
            })
        })
        .clone()
}

/// Draw the Strings / Tag Fields toggle for a string list tag, and the
/// Strings tab when it is the one shown. Returns whether it drew the tab,
/// leaving nothing for the field tree.
pub(super) fn draw_string_list_tag(
    ui: &mut Ui,
    tag: &TagFile,
    document_revision: (u64, u64, u64, u64),
    game: Option<GameId>,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
) -> bool {
    let view_id = edit.widget_id("string list view");
    let mut view: StringListView = ui.data_mut(|data| data.get_temp(view_id)).unwrap_or_default();
    draw_preview_panel_toggle(
        ui,
        &mut view.tab,
        StringListTab::Fields,
        StringListTab::Strings,
        "Strings",
        ButtonIcon::Doc,
    );
    ui.add_space(6.0);
    let drew = view.tab == StringListTab::Strings;
    if drew {
        match cached_list(ui, edit, tag, document_revision, &mut view) {
            Some(cache) => draw_strings(ui, &cache, game, expert_mode, edit, &mut view),
            None => {
                ui.label(RichText::new("This tag's strings could not be read.").color(subtle_dark()));
            }
        }
    }
    ui.data_mut(|data| data.insert_temp(view_id, view));
    drew
}

/// The strings and the ones the filter shows, read again when the tag or the
/// filter changes.
fn cached_list(
    ui: &Ui,
    edit: &FieldEditContext<'_>,
    tag: &TagFile,
    revision: (u64, u64, u64, u64),
    view: &mut StringListView,
) -> Option<Arc<ListCache>> {
    let cache_id = edit.widget_id("string list cache");
    let cached: Option<Arc<ListCache>> = ui.data_mut(|data| data.get_temp(cache_id));
    let list = match &cached {
        Some(cache) if cache.revision == revision => cache.list.clone(),
        _ => Arc::new(strings::read_string_list(tag)?),
    };
    view.language = view.language.min(list.languages.len().saturating_sub(1));
    let filter_key = (view.language, view.filter.trim().to_lowercase());
    if let Some(cache) = &cached
        && cache.revision == revision
        && cache.filter_key == filter_key
    {
        return Some(cache.clone());
    }
    let syntax = text_syntax(edit.definitions_root, edit.game);
    let visible = (0..list.entries.len())
        .filter(|&index| entry_matches_filter(&list.entries[index], view.language, &filter_key.1, &syntax))
        .collect();
    let cache = Arc::new(ListCache { revision, list, filter_key, visible: Arc::new(visible) });
    ui.data_mut(|data| data.insert_temp(cache_id, cache.clone()));
    Some(cache)
}

/// Whether a string's id, or its text as shown, contains `filter` (already
/// lowercase), ignoring case.
fn entry_matches_filter(entry: &StringEntry, language: usize, filter: &str, syntax: &TextSyntax) -> bool {
    if filter.is_empty() {
        return true;
    }
    let id_matches = entry.id.as_deref().is_some_and(|id| id.to_lowercase().contains(filter));
    id_matches
        || entry.texts.get(language).cloned().flatten().is_some_and(|text| {
            strings::format_shown(&text, syntax).to_lowercase().contains(filter)
        })
}

/// What the toolbar and keys asked for this frame, applied after the table
/// so a cell's commit, made against the indices the table drew, goes first.
#[derive(Default)]
struct ToolbarActions {
    add: bool,
    insert: bool,
    duplicate: bool,
    remove: bool,
    confirm_remove: bool,
    copy: bool,
    paste: Option<String>,
}

fn draw_strings(
    ui: &mut Ui,
    cache: &ListCache,
    game: Option<GameId>,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
    view: &mut StringListView,
) {
    let list = &cache.list;
    // Halo CE's syntax has no glyphs; its renderer still reads `|n`.
    let syntax = text_syntax(edit.definitions_root, game);
    let editable = edit.editable;
    view.selected.retain(|&index| index < list.entries.len());
    let mut actions = ToolbarActions::default();

    ui.horizontal(|ui| {
        if list.languages.len() > 1 {
            ui.label("Language:");
            let current = list.languages[view.language].name.clone();
            egui::ComboBox::from_id_salt(edit.widget_id("string list language"))
                .selected_text(current)
                .width(190.0)
                .show_ui(ui, |ui| {
                    for (index, language) in list.languages.iter().enumerate() {
                        ui.selectable_value(&mut view.language, index, &language.name);
                    }
                });
            ui.add_space(12.0);
        }
        ui.label("Filter:");
        ui.add(
            egui::TextEdit::singleline(&mut view.filter)
                .id(edit.widget_id("string list filter"))
                .hint_text("id or text")
                .desired_width(220.0),
        );
        ui.add_space(12.0);
        let shown = cache.visible.len();
        let count = list.entries.len();
        let noun = if count == 1 { "string" } else { "strings" };
        let label = if shown == count { format!("{count} {noun}") } else { format!("{shown} of {count} {noun}") };
        ui.label(RichText::new(label).color(subtle_dark()));
    });
    ui.horizontal(|ui| {
        let any = !view.selected.is_empty();
        actions.add = icon_button(ui, ButtonIcon::Add, "Add a string at the end", editable, text_dark()).clicked();
        actions.insert =
            icon_button(ui, ButtonIcon::InsertRow, "Insert a string before the selected one", editable, text_dark())
                .clicked();
        actions.duplicate =
            icon_button(ui, ButtonIcon::Duplicate, "Duplicate the selected strings", editable && any, text_dark())
                .clicked();
        actions.remove =
            icon_button(ui, ButtonIcon::Garbage, "Remove the selected strings", editable && any, text_dark()).clicked();
        ui.add_space(8.0);
        let copy_tip = match list.storage {
            Storage::Multilingual => "Copy the selected strings as tool [Strings] lines (Ctrl+C)",
            Storage::Classic => "Copy the selected strings as a tool unicode-strings file (Ctrl+C)",
        };
        actions.copy = icon_button(ui, ButtonIcon::Copy, copy_tip, any, text_dark()).clicked();
        if editable {
            ui.label(RichText::new("Paste strings with Ctrl+V").color(subtle_dark()).small());
        }
    });
    if view.confirm_remove {
        ui.horizontal(|ui| {
            let count = view.selected.len();
            ui.label(format!("Remove {count} string{}?", if count == 1 { "" } else { "s" }));
            if ui.button("Remove").clicked() {
                actions.confirm_remove = true;
            }
            if ui.button("Cancel").clicked() {
                view.confirm_remove = false;
            }
        });
    }
    if list.damaged_offsets > 0 {
        ui.label(
            RichText::new(format!(
                "{} text offset{} point past the end of this tag's text, and read as no text. Any edit writes \
                 them as no text.",
                list.damaged_offsets,
                if list.damaged_offsets == 1 { "" } else { "s" }
            ))
            .color(ui.visuals().warn_fg_color),
        );
    }
    ui.add_space(4.0);

    let table_rect = draw_table(ui, cache, &syntax, editable, edit, view);

    // Copy, paste and Delete act on the table when no box has the keyboard.
    let table_hovered = ui.rect_contains_pointer(table_rect);
    if table_hovered && ui.memory(|memory| memory.focused().is_none()) {
        ui.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Copy => actions.copy = !view.selected.is_empty(),
                    egui::Event::Paste(text) if editable => actions.paste = Some(text.clone()),
                    egui::Event::Key { key: egui::Key::Delete, pressed: true, .. } if editable => {
                        actions.remove = !view.selected.is_empty();
                    }
                    _ => {}
                }
            }
        });
    }
    apply_actions(ui, list, &syntax, expert_mode, edit, view, actions);
}

/// Draw the table; returns the rect it took.
fn draw_table(
    ui: &mut Ui,
    cache: &ListCache,
    syntax: &TextSyntax,
    editable: bool,
    edit: &mut FieldEditContext<'_>,
    view: &mut StringListView,
) -> egui::Rect {
    let list = &cache.list;
    let visible = cache.visible.clone();
    let multilingual = list.storage == Storage::Multilingual;
    let language = view.language;
    let scroll_to = view.scroll_to.take().and_then(|index| visible.iter().position(|&shown| shown == index));
    let height = ui.available_height().max(120.0);
    let top = ui.cursor().min;
    let mut clicked: Option<(usize, egui::Modifiers)> = None;
    let mut table = TableBuilder::new(ui)
        .id_salt(edit.widget_id("string list table"))
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(height)
        .min_scrolled_height(0.0)
        .column(Column::exact(56.0));
    if multilingual {
        table = table.column(Column::initial(260.0).at_least(100.0).clip(true));
    }
    table = table.column(Column::remainder().at_least(160.0).clip(true));
    if let Some(row) = scroll_to {
        table = table.scroll_to_row(row, Some(egui::Align::Center));
    }
    table
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.strong("#");
            });
            if multilingual {
                header.col(|ui| {
                    ui.strong("String id");
                });
            }
            header.col(|ui| {
                ui.strong(&list.languages[language].name);
            });
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, visible.len(), |mut row| {
                let index = visible[row.index()];
                let entry = &list.entries[index];
                row.set_selected(view.selected.contains(&index));
                row.col(|ui| {
                    let response = ui.add(egui::Button::selectable(
                        view.selected.contains(&index),
                        RichText::new(index.to_string()).monospace(),
                    ));
                    if response.clicked() {
                        clicked = Some((index, ui.input(|input| input.modifiers)));
                    }
                });
                if multilingual {
                    row.col(|ui| {
                        id_cell(ui, list, index, editable, edit);
                    });
                }
                row.col(|ui| {
                    let text = entry.texts.get(language).cloned().flatten().unwrap_or_default();
                    let shown = strings::format_shown(&text, syntax);
                    let target = TextTarget { index, language, max_units: list.max_text_units(), list };
                    text_cell(ui, &shown, target, syntax, editable, edit, &mut view.completion);
                });
            });
        });
    if let Some((index, modifiers)) = clicked {
        select(view, &visible, index, modifiers);
    }
    egui::Rect::from_min_max(top, ui.min_rect().max.max(top))
}

/// Change the selection for a click on string `index`'s number.
fn select(view: &mut StringListView, visible: &[usize], index: usize, modifiers: egui::Modifiers) {
    if modifiers.shift
        && let Some(anchor) = view.anchor
        && let (Some(from), Some(to)) =
            (visible.iter().position(|&i| i == anchor), visible.iter().position(|&i| i == index))
    {
        let (from, to) = (from.min(to), from.max(to));
        if !modifiers.command {
            view.selected.clear();
        }
        view.selected.extend(&visible[from..=to]);
        return;
    }
    if modifiers.command {
        if !view.selected.remove(&index) {
            view.selected.insert(index);
        }
    } else {
        view.selected = BTreeSet::from([index]);
    }
    view.anchor = Some(index);
    view.confirm_remove = false;
}

fn id_cell(ui: &mut Ui, list: &StringList, index: usize, editable: bool, edit: &mut FieldEditContext<'_>) {
    let current = list.entries[index].id.clone().unwrap_or_default();
    let key = format!("{}|strings|{index}|id", edit.tag_key);
    let id = edit.widget_id(("string id", &key));
    let tag_key = edit.tag_key.to_owned();
    let game = edit.game;
    let draft = edit.buffers.draft_mut_exact(&key, &current);
    let rect = ui.available_rect_before_wrap();
    let background = ui.painter().add(egui::Shape::Noop);
    let response = cell_box(ui, rect, |ui| {
        let mut read_only;
        let buffer: &mut dyn egui::TextBuffer = if editable {
            &mut draft.text
        } else {
            read_only = draft.text.as_str();
            &mut read_only
        };
        egui::TextEdit::singleline(buffer)
            .id(id)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 2)))
            .font(egui::TextStyle::Monospace)
            .vertical_align(egui::Align::Center)
            .desired_width(f32::INFINITY)
            .show(ui)
            .response
            .response
    });
    let problem = id_problem(list, index, &draft.text);
    paint_cell_fill(ui, background, rect, problem.is_some().then(invalid_value_fill));
    let response = match &problem {
        Some(problem) => response.on_hover_text(problem.as_str()),
        None => response,
    };
    draft.note_response(&response);
    let mut status = None;
    if !valid_id(&draft.text) {
        status = keep_refused_draft(ui, &response, draft, &current);
    } else if editable && draft.should_commit(ui, &response) {
        edit.string_list_ops.push(StringListOp { game: edit.game, edit: StringListEdit::SetId { index, id: draft.text.clone() } });
    }
    draft.keep_commit(|| {
        DraftCommit::new(&tag_key, vec![key.clone()], move |texts| {
            let id = texts[0];
            if !valid_id(id) {
                return Err(format!("string {index}: an id can't be empty or start or end with a space"));
            }
            Ok(DeferredOps {
                string_list_ops: vec![StringListOp { game, edit: StringListEdit::SetId { index, id: id.to_owned() } }],
                ..DeferredOps::default()
            })
        })
    });
    if let Some(status) = status {
        set_status(edit, status);
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.trim() == id
}

/// What is wrong with `id` as string `index`'s id, if anything.
fn id_problem(list: &StringList, index: usize, id: &str) -> Option<String> {
    if !valid_id(id) {
        return Some("An id can't be empty, or start or end with a space.".to_owned());
    }
    let others = list
        .entries
        .iter()
        .enumerate()
        .filter(|&(other, entry)| other != index && entry.id.as_deref() == Some(id))
        .count();
    (others > 0).then(|| {
        format!(
            "{} other string{} this id. Importing text into the tag sets one string per id.",
            others,
            if others == 1 { " has" } else { "s have" }
        )
    })
}

/// Which text a cell edits, and the list it is in.
#[derive(Clone, Copy)]
struct TextTarget<'a> {
    index: usize,
    language: usize,
    max_units: usize,
    list: &'a StringList,
}

fn text_cell(
    ui: &mut Ui,
    shown: &str,
    target: TextTarget<'_>,
    syntax: &TextSyntax,
    editable: bool,
    edit: &mut FieldEditContext<'_>,
    completion: &mut CompletionView,
) {
    let TextTarget { index, language, max_units, list } = target;
    let key = format!("{}|strings|{index}|{language}", edit.tag_key);
    let id = edit.widget_id(("string text", &key));
    let filter_id = id.with("completion filter");
    let tag_key = edit.tag_key.to_owned();
    let game = edit.game;
    let draft = edit.buffers.draft_mut_exact(&key, shown);
    let ours = completion.cell == key;
    let focused = ui.memory(|memory| memory.has_focus(id));
    let filter_focused = ours && ui.memory(|memory| memory.has_focus(filter_id));

    // Escape in the list's filter box: egui has already taken focus from
    // it, so put it back in the cell, with the list closed.
    if ours && completion.filter_focused && !filter_focused && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        if let Some(open) = &completion.open {
            completion.dismissed_at = Some(open.replace.start);
        }
        completion.close();
        ui.memory_mut(|memory| memory.request_focus(id));
    }

    // The list takes the arrow keys, Enter, Tab and Escape before the cell or
    // its filter box sees them.
    if editable && (focused || filter_focused) {
        let open = if focused {
            let cursor = cursor_byte(ui, id, &draft.text);
            open_completion(syntax, &draft.text, cursor, completion, &key)
        } else {
            completion.open.clone()
        };
        if let Some(open) = open {
            let choices = filtered_choices(syntax, open.kind, &completion.filter);
            let count = choices.len();
            let mut accepted = None;
            let mut dismissed = false;
            ui.input_mut(|input| {
                if count > 0 && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    completion.highlighted = (completion.highlighted + 1) % count;
                    completion.scroll = true;
                }
                if count > 0 && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    completion.highlighted = (completion.highlighted + count - 1) % count;
                    completion.scroll = true;
                }
                if count > 0
                    && (input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
                {
                    accepted = Some(choices[completion.highlighted.min(count - 1)].insert.clone());
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    dismissed = true;
                }
            });
            if let Some(insert) = accepted {
                insert_completion(ui, id, &mut draft.text, &open, &insert);
                draft.changed = true;
                completion.close();
                ui.memory_mut(|memory| memory.request_focus(id));
            } else if dismissed {
                completion.dismissed_at = Some(open.replace.start);
                completion.close();
                ui.memory_mut(|memory| memory.request_focus(id));
            }
        }
    }

    let rect = ui.available_rect_before_wrap();
    let font_id = ui.style().text_styles[&TextStyle::Monospace].clone();
    let colors = TokenColors::new();
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, _wrap_width: f32| {
        let analysis = strings::analyze(buffer.as_str(), syntax);
        let unpaired = unpaired_bindings(buffer.as_str(), &analysis, list, game);
        ui.fonts_mut(|fonts| fonts.layout_job(highlighted_job(buffer.as_str(), &analysis, &unpaired, &font_id, &colors)))
    };
    let background = ui.painter().add(egui::Shape::Noop);
    let output = cell_box_output(ui, rect, |ui| {
        let mut read_only;
        let buffer: &mut dyn egui::TextBuffer = if editable {
            &mut draft.text
        } else {
            read_only = draft.text.as_str();
            &mut read_only
        };
        egui::TextEdit::singleline(buffer)
            .id(id)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 2)))
            .vertical_align(egui::Align::Center)
            .desired_width(f32::INFINITY)
            .hint_text(RichText::new("no text").italics().color(subtle_dark()))
            .layouter(&mut layouter)
            .show(ui)
    });
    // Judged on the text as this frame's typing left it.
    let analysis = strings::analyze(&draft.text, syntax);
    let errors = analysis.has_errors();
    let text = analysis.text.clone();
    let units = text.encode_utf16().count();
    let too_long = units > max_units;
    paint_cell_fill(ui, background, rect, (errors || too_long).then(invalid_value_fill));
    let response = output.response.response.clone();
    let response = if errors {
        response.on_hover_text("The tool would refuse this text: a literal & is &&, and a glyph is &name or &U+XXXX.")
    } else if too_long {
        response.on_hover_text(format!("{units} characters: the tool keeps only the first {max_units}."))
    } else {
        match markup_hover(&draft.text, &analysis, &unpaired_bindings(&draft.text, &analysis, list, game)) {
            Some(hover) => response.on_hover_text(hover),
            None => response,
        }
    };
    draft.note_response(&response);

    // Focus that goes to the list, by a click on a choice or into its filter
    // box, must not commit what the cell holds.
    let filter_has_focus = ui.memory(|memory| memory.has_focus(filter_id));
    let to_list = ours && response.lost_focus() && (completion.hovered || filter_has_focus);
    let mut status = None;
    if to_list {
        if !filter_has_focus {
            response.request_focus();
        }
    } else if errors {
        status = keep_refused_draft(ui, &response, draft, shown);
    } else if editable && draft.should_commit(ui, &response) {
        edit.string_list_ops.push(StringListOp { game, edit: StringListEdit::SetText { index, language, text: text.clone() } });
    }
    draft.keep_commit(|| {
        let syntax = syntax.clone();
        DraftCommit::new(&tag_key, vec![key.clone()], move |texts| {
            let text = strings::parse_shown(texts[0], &syntax)
                .map_err(|_| format!("string {index}: the tool would refuse its text"))?;
            Ok(DeferredOps {
                string_list_ops: vec![StringListOp { game, edit: StringListEdit::SetText { index, language, text } }],
                ..DeferredOps::default()
            })
        })
    });

    // A swatch in the gap the layout left before each colour; a click opens
    // the colour picker on it.
    let mut colour_request = None;
    let mut colours_in_text = strings::scan_markup(&text, syntax.markup()).into_iter().filter_map(|span| span.color);
    for (n, span) in analysis.markup.iter().filter(|span| span.color.is_some()).enumerate() {
        let colour = span.color.as_ref().expect("filtered");
        let at = draft.text[..span.range.start].chars().count();
        let glyph = output.galley.pos_from_cursor(egui::text::CCursor::new(at)).translate(output.galley_pos.to_vec2());
        let swatch = egui::Rect::from_center_size(
            egui::pos2(glyph.left() - SWATCH_SPACE / 2.0, glyph.center().y),
            Vec2::splat(SWATCH_SIZE),
        );
        paint_swatch(&ui.painter().with_clip_rect(output.text_clip_rect), swatch, colour.argb);
        let in_text = colours_in_text.next();
        if !editable || !output.text_clip_rect.intersects(swatch) {
            continue;
        }
        let clicked = ui
            .interact(swatch, id.with(("colour", n)), Sense::click())
            .on_hover_text("Pick this colour")
            .clicked();
        if clicked && let Some(in_text) = in_text {
            // What is typed goes to the tag first, so the picked colour lands
            // on it.
            if draft.changed && !errors {
                edit.string_list_ops.push(StringListOp { game, edit: StringListEdit::SetText { index, language, text: text.clone() } });
                draft.mark_committed();
            }
            colour_request = Some(crate::app::editor::StringListColourWrite { game, index, language, text: text.clone(), colour: in_text });
        }
    }

    // The list itself, under the cell, while the cursor follows an `&` or a
    // `#`, or its filter box is being typed in.
    let mut list_open = false;
    let cell_focused = response.has_focus();
    if editable && (cell_focused || filter_has_focus) {
        let open = if cell_focused {
            let cursor = output
                .cursor_range
                .map(|range| byte_of_char(&draft.text, range.primary.index.0))
                .unwrap_or(draft.text.len());
            open_completion(syntax, &draft.text, cursor, completion, &key)
        } else {
            completion.open.clone()
        };
        match open {
            Some(open) => {
                if completion.cell != key || completion.open.as_ref() != Some(&open) {
                    let reopened = completion.cell != key
                        || completion.open.as_ref().is_none_or(|last| last.replace.start != open.replace.start);
                    if reopened {
                        completion.filter.clear();
                    }
                    completion.cell = key.clone();
                    let choices = filtered_choices(syntax, open.kind, &completion.filter);
                    completion.highlighted =
                        strings::best_match(choices.iter().map(|choice| choice.name.as_str()), &open.typed).unwrap_or(0);
                    completion.scroll = true;
                    completion.open = Some(open.clone());
                }
                list_open = true;
                if let Some(insert) = draw_completion_list(ui, id, filter_id, response.rect, syntax, &open, completion) {
                    insert_completion(ui, id, &mut draft.text, &open, &insert);
                    draft.changed = true;
                    completion.close();
                    ui.memory_mut(|memory| memory.request_focus(id));
                }
            }
            None => {
                if completion.cell == key {
                    completion.close();
                }
            }
        }
    } else if ours && !to_list {
        completion.close();
    }
    if completion.cell == key {
        completion.filter_focused = ui.memory(|memory| memory.has_focus(filter_id));
    }
    // egui takes focus away on Escape, and moves it on Tab, before any widget
    // sees the key, unless the focused widget claims it. The cell claims both
    // while its list is open.
    if response.has_focus() {
        let filter = egui::EventFilter {
            horizontal_arrows: true,
            vertical_arrows: true,
            tab: list_open,
            escape: list_open,
        };
        ui.memory_mut(|memory| memory.set_focus_lock_filter(id, filter));
    }
    if let Some(status) = status {
        set_status(edit, status);
    }
    if let Some(write) = colour_request {
        *edit.color_request = Some(MaterialColorPopup::for_string_list_colour(tag_key, write));
    }
}

/// A colour's swatch: the colour over a checkerboard, so a translucent one
/// shows as such, with an edge.
fn paint_swatch(painter: &egui::Painter, rect: egui::Rect, argb: u32) {
    let [a, r, g, b] = argb.to_be_bytes();
    let half = rect.width() / 2.0;
    for (x, y) in [(0.0, 0.0), (half, 0.0), (0.0, half), (half, half)] {
        let light = (x == 0.0) == (y == 0.0);
        let cell = egui::Rect::from_min_size(rect.min + Vec2::new(x, y), Vec2::splat(half));
        painter.rect_filled(cell, 0.0, if light { Color32::from_gray(230) } else { Color32::from_gray(160) });
    }
    painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(r, g, b, a));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, text_dark()), egui::StrokeKind::Inside);
}

/// What to do with a box whose text can't be committed as it loses focus:
/// Escape puts back the tag's value, Enter keeps the box focused so the text
/// can be fixed. Returns a status line for Enter.
fn keep_refused_draft(ui: &Ui, response: &egui::Response, draft: &mut EditDraft, value: &str) -> Option<String> {
    if !response.lost_focus() {
        return None;
    }
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        draft.set_clean(value);
        return None;
    }
    if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
        response.request_focus();
        return Some("The tool would refuse this text, so it isn't saved: fix the part marked in red.".to_owned());
    }
    None
}

/// The glyph or keyword being typed at byte `cursor`, unless Escape closed
/// its list or it is a whole name already.
fn open_completion(
    syntax: &TextSyntax,
    shown: &str,
    cursor: usize,
    completion: &mut CompletionView,
    key: &str,
) -> Option<strings::Completion> {
    let found = strings::completion_at(shown, cursor, syntax);
    if completion.cell == key {
        match (&found, completion.dismissed_at) {
            (Some(found), Some(at)) if found.replace.start == at => return None,
            (_, Some(_)) => completion.dismissed_at = None,
            _ => {}
        }
    }
    let found = found?;
    let finished = strings::choices(syntax, found.kind).iter().any(|choice| choice.name.eq_ignore_ascii_case(&found.typed));
    (!finished).then_some(found)
}

/// The game's choices whose labels contain `filter`, ignoring case.
fn filtered_choices(syntax: &TextSyntax, kind: strings::CompletionKind, filter: &str) -> Vec<strings::Choice> {
    let filter = filter.trim().to_lowercase();
    strings::choices(syntax, kind)
        .into_iter()
        .filter(|choice| filter.is_empty() || choice.label.to_lowercase().contains(&filter))
        .collect()
}

/// Draw the list under `cell`: every choice the game has, the best match
/// highlighted, and a filter box over them when they don't all fit. Returns
/// what to insert for the choice clicked.
fn draw_completion_list(
    ui: &Ui,
    id: egui::Id,
    filter_id: egui::Id,
    cell: egui::Rect,
    syntax: &TextSyntax,
    open: &strings::Completion,
    completion: &mut CompletionView,
) -> Option<String> {
    let mut picked = None;
    let total = strings::choices(syntax, open.kind).len();
    let area = egui::Area::new(id.with("completion list"))
        .order(egui::Order::Foreground)
        .fixed_pos(cell.left_bottom() + Vec2::new(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(cell.width().clamp(260.0, 420.0));
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                let mut filter_changed = false;
                if total > COMPLETION_ROWS {
                    let search = ui.push_id(filter_id, |ui| {
                        let edit = egui::TextEdit::singleline(&mut completion.filter)
                            .id(filter_id)
                            .hint_text(placeholder_text("Filter…"))
                            .desired_width(ui.available_width());
                        ui.add(edit)
                    });
                    filter_changed = search.inner.changed();
                    ui.separator();
                }
                let choices = filtered_choices(syntax, open.kind, &completion.filter);
                if filter_changed {
                    completion.highlighted =
                        strings::best_match(choices.iter().map(|choice| choice.name.as_str()), &open.typed).unwrap_or(0);
                    completion.scroll = true;
                }
                let scroll = std::mem::take(&mut completion.scroll);
                picker_results(ui, ROW_HEIGHT * COMPLETION_ROWS as f32, filter_changed, |ui| {
                    for (row, choice) in choices.iter().enumerate() {
                        let highlighted = row == completion.highlighted;
                        let button = egui::Button::selectable(highlighted, RichText::new(&choice.label).monospace())
                            .right_text(RichText::new(&choice.detail).color(subtle_dark()).small())
                            .min_size(Vec2::new(ui.available_width(), 0.0));
                        let response = ui.add(button);
                        if highlighted && scroll {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() {
                            picked = Some(choice.insert.clone());
                        }
                    }
                    if choices.is_empty() {
                        ui.label(RichText::new("Nothing matches.").color(subtle_dark()));
                    }
                });
            });
        });
    completion.hovered = area.response.contains_pointer();
    picked
}

/// Replace what is being typed with the chosen name, and put the cursor
/// after it.
fn insert_completion(ui: &Ui, id: egui::Id, text: &mut String, completion: &strings::Completion, inserted: &str) {
    text.replace_range(completion.replace.clone(), inserted);
    let cursor = text[..completion.replace.start].chars().count() + inserted.chars().count();
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
    egui::TextEdit::store_state(ui.ctx(), id, state);
}

/// The byte the box's cursor is at, as it was left last frame.
fn cursor_byte(ui: &Ui, id: egui::Id, text: &str) -> usize {
    egui::TextEdit::load_state(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range())
        .map_or(text.len(), |range| byte_of_char(text, range.primary.index.0))
}

fn byte_of_char(text: &str, char_index: usize) -> usize {
    text.char_indices().nth(char_index).map_or(text.len(), |(at, _)| at)
}

/// The colors a cell draws its text in.
struct TokenColors {
    text: Color32,
    escape: Color32,
    glyph: Color32,
    keyword: Color32,
    tag: Color32,
    comment: Color32,
    warning: Color32,
    error: Color32,
}

impl TokenColors {
    fn new() -> Self {
        Self {
            text: text_dark(),
            escape: subtle_dark(),
            glyph: foundation_blue(),
            keyword: good_news(),
            tag: markup_tag_text(),
            comment: subtle_dark(),
            warning: markup_warning_text(),
            error: invalid_value_text(),
        }
    }
}

/// The bindings in `analysis` that have no substitution pair, where the game
/// needs one and its tool's rule for making them isn't known, so the editor
/// can't make it (Halo 4, Halo 2 Anniversary). Reach's pairs are rebuilt on
/// every edit.
fn unpaired_bindings(shown: &str, analysis: &strings::Analysis, list: &StringList, game: Option<GameId>) -> Vec<Range<usize>> {
    if !matches!(game, Some(GameId::Halo4 | GameId::Halo2Amp)) {
        return Vec::new();
    }
    analysis
        .markup
        .iter()
        .filter(|span| span.kind == strings::MarkupKind::Binding && !span.warning)
        .filter(|span| {
            strings::cui_bindings(&shown[span.range.clone()])
                .first()
                .is_some_and(|(component, property)| !list.has_pair(component, property))
        })
        .map(|span| span.range.clone())
        .collect()
}

/// What the cell's hover says about its markup: warnings first, then what
/// each kind of markup does.
fn markup_hover(shown: &str, analysis: &strings::Analysis, unpaired: &[Range<usize>]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut add = |line: String| {
        if !lines.contains(&line) {
            lines.push(line);
        }
    };
    for span in analysis.markup.iter().filter(|span| span.warning) {
        add(format!("⚠ {}: {}", &shown[span.range.clone()], span.note.as_deref().unwrap_or_default()));
    }
    for range in unpaired {
        add(format!(
            "⚠ {}: this list has no substitution pair for it, so the game shows nothing there",
            &shown[range.clone()]
        ));
    }
    for span in analysis.markup.iter().filter(|span| !span.warning) {
        if let Some(note) = &span.note {
            add(format!("{}: {note}", &shown[span.range.clone()]));
        }
    }
    lines.truncate(8);
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn highlighted_job(
    shown: &str,
    analysis: &strings::Analysis,
    unpaired: &[Range<usize>],
    font_id: &egui::FontId,
    colors: &TokenColors,
) -> egui::text::LayoutJob {
    // Cut the text wherever a token or a piece of markup starts or ends,
    // and style each cut by what covers it.
    let mut cuts: Vec<usize> = vec![0, shown.len()];
    for token in &analysis.tokens {
        cuts.extend([token.range.start, token.range.end]);
    }
    for span in &analysis.markup {
        cuts.extend([span.range.start, span.range.end]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut job = egui::text::LayoutJob::default();
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end || !shown.is_char_boundary(start) || !shown.is_char_boundary(end) {
            continue;
        }
        let mut format = egui::TextFormat::simple(font_id.clone(), colors.text);
        let token = analysis.tokens.iter().find(|token| token.range.start <= start && start < token.range.end);
        match token.map(|token| token.kind) {
            Some(TokenKind::Escape | TokenKind::Ampersand) => format.color = colors.escape,
            Some(TokenKind::Glyph) => format.color = colors.glyph,
            _ => {}
        }
        if let Some(span) = analysis.markup.iter().find(|span| span.range.start <= start && start < span.range.end) {
            format.color = match span.kind {
                strings::MarkupKind::Escape | strings::MarkupKind::Color | strings::MarkupKind::Entity => colors.escape,
                strings::MarkupKind::Keyword | strings::MarkupKind::Placeholder | strings::MarkupKind::Binding => colors.keyword,
                strings::MarkupKind::Tag => colors.tag,
                strings::MarkupKind::Comment => colors.comment,
            };
            if span.kind == strings::MarkupKind::Comment {
                format.italics = true;
            }
            if span.warning || unpaired.iter().any(|range| range.start <= start && start < range.end) {
                format.underline = Stroke::new(1.0, colors.warning);
            }
        }
        if token.is_some_and(|token| token.kind.is_error()) {
            format.color = colors.error;
            format.underline = Stroke::new(1.0, colors.error);
        }
        let swatch = analysis.markup.iter().any(|span| span.color.is_some() && span.range.start == start);
        job.append(&shown[start..end], if swatch { SWATCH_SPACE } else { 0.0 }, format);
    }
    job
}

/// A cell's box: its edge, and what `contents` draws in it. Its fill is
/// painted after, by [`paint_cell_fill`], once the cell knows whether what
/// was typed this frame is valid.
fn cell_box(ui: &mut Ui, rect: egui::Rect, contents: impl FnOnce(&mut Ui) -> egui::Response) -> egui::Response {
    cell_box_output(ui, rect, contents)
}

fn cell_box_output<R: CellOutput>(ui: &mut Ui, rect: egui::Rect, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = cell_rect(rect);
    let output = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), contents).inner;
    let stroke = if output.focused() {
        ui.visuals().selection.stroke
    } else {
        Stroke::new(1.0_f32, foundation_input_edge())
    };
    ui.painter().rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
    output
}

fn cell_rect(rect: egui::Rect) -> egui::Rect {
    rect.shrink2(Vec2::new(0.0, 1.0))
}

/// Fill the shape reserved under a cell's box.
fn paint_cell_fill(ui: &Ui, background: egui::layers::ShapeIdx, rect: egui::Rect, fill: Option<Color32>) {
    let shape = egui::Shape::rect_filled(cell_rect(rect), 0.0, fill.unwrap_or(foundation_input()));
    ui.painter().set(background, shape);
}

/// What a cell's contents return, for its box to draw focus around.
trait CellOutput {
    fn focused(&self) -> bool;
}

impl CellOutput for egui::Response {
    fn focused(&self) -> bool {
        self.has_focus()
    }
}

impl CellOutput for egui::text_edit::TextEditOutput {
    fn focused(&self) -> bool {
        self.response.response.has_focus()
    }
}

/// Apply what the toolbar and keys asked for.
fn apply_actions(
    ui: &Ui,
    list: &StringList,
    syntax: &TextSyntax,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
    view: &mut StringListView,
    actions: ToolbarActions,
) {
    let count = list.entries.len();
    let first = view.selected.first().copied();
    let last = view.selected.last().copied();
    if actions.add {
        insert_entries(edit, view, count, vec![list.new_entry()]);
    }
    if actions.insert {
        insert_entries(edit, view, first.unwrap_or(0), vec![list.new_entry()]);
    }
    if actions.duplicate && let Some(last) = last {
        let mut taken: BTreeSet<String> = list.entries.iter().filter_map(|entry| entry.id.clone()).collect();
        let copies = view
            .selected
            .iter()
            .map(|&index| {
                let mut entry = list.entries[index].clone();
                entry.id = entry.id.map(|id| unused_copy_id(&id, &mut taken));
                entry
            })
            .collect();
        insert_entries(edit, view, last + 1, copies);
    }
    if actions.remove && !view.selected.is_empty() {
        if expert_mode {
            remove_selected(edit, view);
        } else {
            view.confirm_remove = true;
        }
    }
    if actions.confirm_remove {
        remove_selected(edit, view);
    }
    if actions.copy {
        let selected = view.selected.iter().map(|&index| &list.entries[index]);
        let text = match list.storage {
            Storage::Multilingual => selected
                .map(|entry| {
                    let text = entry.texts.get(view.language).cloned().flatten().unwrap_or_default();
                    strings::strings_file_line(entry.id.as_deref().unwrap_or_default(), &text, syntax) + "\r\n"
                })
                .collect::<String>(),
            Storage::Classic => {
                let texts: Vec<String> = selected.map(|entry| entry.texts[0].clone().unwrap_or_default()).collect();
                strings::ce_strings_file(texts.iter().map(String::as_str))
            }
        };
        ui.ctx().copy_text(text);
        set_status(edit, format!("Copied {} strings", view.selected.len()));
    }
    if let Some(pasted) = actions.paste {
        paste(list, syntax, edit, view, &pasted);
    }
}

/// `id` with `_copy` (or `_copy_2`, …) added, unused by any id in `taken`,
/// which it joins.
fn unused_copy_id(id: &str, taken: &mut BTreeSet<String>) -> String {
    let copy = std::iter::once(format!("{id}_copy"))
        .chain((2..).map(|n| format!("{id}_copy_{n}")))
        .find(|candidate| !taken.contains(candidate))
        .expect("unbounded");
    taken.insert(copy.clone());
    copy
}

fn insert_entries(edit: &mut FieldEditContext<'_>, view: &mut StringListView, at: usize, entries: Vec<StringEntry>) {
    let count = entries.len();
    edit.string_list_ops.push(StringListOp { game: edit.game, edit: StringListEdit::Insert { at, entries } });
    view.selected = (at..at + count).collect();
    view.anchor = Some(at);
    view.scroll_to = Some(at);
    view.confirm_remove = false;
}

fn remove_selected(edit: &mut FieldEditContext<'_>, view: &mut StringListView) {
    let indices: Vec<usize> = view.selected.iter().copied().collect();
    let first = indices.first().copied();
    edit.string_list_ops.push(StringListOp { game: edit.game, edit: StringListEdit::Remove { indices } });
    view.selected.clear();
    view.confirm_remove = false;
    view.anchor = None;
    view.scroll_to = first;
}

/// Paste tool text: `[Strings]` lines set the current language's text of the
/// strings with those ids, and add the rest after the selection; Halo CE's
/// `###END-STRING###` text adds strings after the selection.
fn paste(list: &StringList, syntax: &TextSyntax, edit: &mut FieldEditContext<'_>, view: &mut StringListView, pasted: &str) {
    let at = view.selected.last().map_or(list.entries.len(), |&last| last + 1);
    match list.storage {
        Storage::Classic => {
            let entries: Vec<StringEntry> = strings::parse_ce_strings_file(pasted)
                .into_iter()
                .map(|text| StringEntry { id: None, texts: vec![Some(text)] })
                .collect();
            if entries.is_empty() {
                return;
            }
            let count = entries.len();
            insert_entries(edit, view, at, entries);
            set_status(edit, format!("Pasted {count} strings"));
        }
        Storage::Multilingual => {
            let parsed = match strings::parse_strings_file(pasted, syntax) {
                Ok(parsed) if !parsed.is_empty() => parsed,
                Ok(_) => return,
                Err(error) => {
                    set_status(edit, format!("Can't paste strings: {error}"));
                    return;
                }
            };
            let language = view.language;
            let mut added = Vec::new();
            let mut updated = 0;
            for (id, text) in parsed {
                match list.entries.iter().position(|entry| entry.id.as_deref() == Some(id.as_str())) {
                    Some(index) => {
                        edit.string_list_ops.push(StringListOp { game: edit.game, edit: StringListEdit::SetText { index, language, text } });
                        updated += 1;
                    }
                    None => {
                        let mut texts = vec![None; list.languages.len()];
                        texts[language] = Some(text);
                        added.push(StringEntry { id: Some(id), texts });
                    }
                }
            }
            let added_count = added.len();
            if !added.is_empty() {
                insert_entries(edit, view, at, added);
            }
            set_status(edit, format!("Pasted strings: {updated} updated, {added_count} added"));
        }
    }
}

fn set_status(edit: &mut FieldEditContext<'_>, message: String) {
    if let Some(status) = edit.status.as_deref_mut() {
        *status = message;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::bundled::locate_definitions_root;

    /// A Halo 3 string list holding `texts` as English, ids `s0`, `s1`, …
    fn string_list_tag(texts: &[&str]) -> TagFile {
        let path = locate_definitions_root().join("halo3_mcc/multilingual_unicode_string_list.json");
        let mut tag = TagFile::new(path).unwrap();
        let entries = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let mut texts = vec![None; 12];
                texts[0] = Some((*text).to_owned());
                StringEntry { id: Some(format!("s{index}")), texts }
            })
            .collect();
        strings::apply_string_list_edits(&mut tag, None, &[StringListEdit::Insert { at: 0, entries }]).unwrap();
        tag
    }

    /// Draws the Strings tab over frames on one context, as a pane does.
    struct Panel {
        ctx: egui::Context,
        tag: TagFile,
        sinks: EditSinks,
        editable: bool,
        time: f64,
        /// The id of string 0's English text box.
        text_id: Option<egui::Id>,
    }

    struct Frame {
        texts: Vec<String>,
        pink: bool,
        edits: Vec<StringListEdit>,
        /// Rects painted, with their fills.
        rects: Vec<(egui::Rect, Color32)>,
        /// The colour picker a click asked for.
        colour_picker: Option<MaterialColorPopup>,
    }

    impl Panel {
        fn new(texts: &[&str]) -> Self {
            Self {
                ctx: egui::Context::default(),
                tag: string_list_tag(texts),
                sinks: EditSinks::default(),
                editable: true,
                time: 0.0,
                text_id: None,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> Frame {
            self.time += 0.1;
            let mut edits = Vec::new();
            let mut text_id = None;
            let mut colour_picker = None;
            let (tag, sinks, editable) = (&self.tag, &mut self.sinks, self.editable);
            let output = crate::app::run_ui_test(&self.ctx, crate::app::loose_fixture::screen(events, self.time), |ui| {
                let mut edit = FieldEditContext::read_only(sinks, "test", "strings.multilingual_unicode_string_list");
                edit.root = Some(tag.root());
                edit.game = Some(GameId::Halo3);
                edit.editable = editable;
                edit.buffers.begin_pass(ui.ctx().cumulative_pass_nr());
                draw_string_list_tag(ui, tag, (1, 1, 1, 1), Some(GameId::Halo3), false, &mut edit);
                assert!(edit.string_list_ops.iter().all(|op| op.game == Some(GameId::Halo3)), "each op names its game");
                edits.extend(edit.string_list_ops.drain(..).map(|op| op.edit));
                let key = format!("{}|strings|0|0", edit.tag_key);
                text_id = Some(edit.widget_id(("string text", &key)));
                colour_picker = edit.color_request.take();
            });
            self.text_id = text_id;
            let mut texts = Vec::new();
            let mut pink = false;
            let mut rects = Vec::new();
            for clipped in &output.shapes {
                match &clipped.shape {
                    egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                    egui::Shape::Rect(rect) => {
                        pink |= rect.fill == invalid_value_fill();
                        rects.push((rect.rect, rect.fill));
                    }
                    _ => {}
                }
            }
            Frame { texts, pink, edits, rects, colour_picker }
        }

        /// Focus string 0's text, with the cursor at its end.
        fn focus_first_text(&mut self, chars: usize) {
            let id = self.text_id.expect("drawn");
            self.ctx.memory_mut(|memory| memory.request_focus(id));
            let mut state = egui::TextEdit::load_state(&self.ctx, id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(chars))));
            egui::TextEdit::store_state(&self.ctx, id, state);
        }
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    fn text(text: &str) -> egui::Event {
        egui::Event::Text(text.to_owned())
    }

    #[test]
    fn glyphs_show_by_name_and_line_breaks_escaped() {
        let mut panel = Panel::new(&["Press \u{E100}\r\nto go", "News & Updates", "\u{E45B}", "\u{E453}_forward"]);
        let frame = panel.frame(Vec::new());
        // Every glyph by its name; a name followed by text that would make a
        // longer one by code point.
        for shown in ["Press &a_button\\r\\nto go", "News && Updates", "&stick_move_forward", "&U+E453_forward", "English", "4 strings"] {
            assert!(frame.texts.iter().any(|text| text == shown), "{shown:?} not in {:?}", frame.texts);
        }
        assert!(!frame.pink, "every text reads back");
    }

    #[test]
    fn typing_an_ampersand_offers_glyph_names_and_enter_inserts_one() {
        let mut panel = Panel::new(&["Press "]);
        panel.frame(Vec::new());
        panel.focus_first_text(6);
        panel.frame(Vec::new());
        assert!(panel.frame(vec![text("&a_b")]).pink, "&a_b alone is not a glyph yet");
        // A new list is measured on its first frame and shown on the next.
        let typed = panel.frame(Vec::new());
        assert!(typed.texts.iter().any(|text| text == "&a_button"), "the list offers a_button: {:?}", typed.texts);
        // Enter takes the highlighted name instead of committing the box.
        let picked = panel.frame(vec![key(egui::Key::Enter)]);
        assert!(picked.edits.is_empty(), "the pick did not commit: {:?}", picked.edits);
        assert!(picked.texts.iter().any(|text| text == "Press &a_button"), "{:?}", picked.texts);
        assert!(!picked.pink);
        // The next Enter commits the text the box now holds.
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(
            committed.edits,
            vec![StringListEdit::SetText { index: 0, language: 0, text: "Press \u{E100}".to_owned() }]
        );
    }

    /// Opens the list after `typed`, typed at the end of string 0.
    fn open_list(panel: &mut Panel, start: usize, typed: &str) -> Frame {
        panel.frame(Vec::new());
        panel.focus_first_text(start);
        panel.frame(Vec::new());
        panel.frame(vec![text(typed)]);
        panel.frame(Vec::new())
    }

    #[test]
    fn the_list_offers_every_name_with_the_best_match_highlighted() {
        let mut panel = Panel::new(&["Press "]);
        let shown = open_list(&mut panel, 6, "&a_");
        // Every glyph the game has, not only those matching what was typed.
        for name in ["&a_button", "&dpad_up", "&back_button"] {
            assert!(shown.texts.iter().any(|text| text == name), "{name} offered: {:?}", shown.texts);
        }
        assert!(shown.texts.iter().any(|text| text == "Filter…"), "a list that scrolls has a filter box");
        assert!(!shown.texts.iter().any(|text| text.contains("Esc close")), "no help line");
        let picked = panel.frame(vec![key(egui::Key::Enter)]);
        assert!(picked.texts.iter().any(|text| text == "Press &a_button"), "{:?}", picked.texts);
    }

    #[test]
    fn the_filter_box_narrows_the_list_and_enter_there_inserts_into_the_cell() {
        let mut panel = Panel::new(&["Press "]);
        open_list(&mut panel, 6, "&");
        let cell = panel.text_id.unwrap();
        let filter = cell.with("completion filter");
        panel.ctx.memory_mut(|memory| memory.request_focus(filter));
        panel.frame(Vec::new());
        let narrowed = panel.frame(vec![text("dpad_d")]);
        assert!(narrowed.texts.iter().any(|text| text == "&dpad_down"), "{:?}", narrowed.texts);
        assert!(!narrowed.texts.iter().any(|text| text == "&a_button"), "filtered out");
        assert!(narrowed.edits.is_empty(), "moving into the filter box commits nothing");
        let picked = panel.frame(vec![key(egui::Key::Enter)]);
        assert!(picked.texts.iter().any(|text| text == "Press &dpad_down"), "{:?}", picked.texts);
        panel.frame(Vec::new());
        assert_eq!(panel.ctx.memory(|memory| memory.focused()), Some(cell), "back in the cell");
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(
            committed.edits,
            vec![StringListEdit::SetText { index: 0, language: 0, text: "Press \u{E109}".to_owned() }]
        );
    }

    /// Unlike a half-typed glyph, a half-typed keyword is valid text, so
    /// moving into the filter box must not commit it.
    #[test]
    fn moving_into_the_filter_box_commits_nothing() {
        let mut panel = Panel::new(&["Win by "]);
        open_list(&mut panel, 7, "#sc");
        let cell = panel.text_id.unwrap();
        panel.ctx.memory_mut(|memory| memory.request_focus(cell.with("completion filter")));
        let moved = panel.frame(Vec::new());
        assert!(moved.edits.is_empty(), "{:?}", moved.edits);
        let narrowed = panel.frame(vec![text("to_w")]);
        assert!(narrowed.edits.is_empty(), "{:?}", narrowed.edits);
        panel.frame(vec![key(egui::Key::Enter)]);
        panel.frame(Vec::new());
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(
            committed.edits,
            vec![StringListEdit::SetText { index: 0, language: 0, text: "Win by #score_to_win".to_owned() }]
        );
    }

    #[test]
    fn an_angle_bracket_offers_the_games_tags() {
        let mut panel = Panel::new(&["Go "]);
        let shown = open_list(&mut panel, 3, "<col");
        assert!(shown.texts.iter().any(|text| text == "<color argb=#AARRGGBB>"), "{:?}", shown.texts);
        panel.frame(vec![key(egui::Key::Enter)]);
        panel.frame(vec![text("x")]);
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(
            committed.edits,
            vec![StringListEdit::SetText { index: 0, language: 0, text: "Go <color argb=#FFFFFFFF>x".to_owned() }]
        );
    }

    #[test]
    fn a_pipe_offers_the_games_escapes() {
        let mut panel = Panel::new(&["Line"]);
        let shown = open_list(&mut panel, 4, "|");
        for label in ["|n", "|kAARRGGBB", "|j"] {
            assert!(shown.texts.iter().any(|text| text == label), "{label}: {:?}", shown.texts);
        }
        assert!(shown.texts.iter().any(|text| text == "align right"), "|r is described as what it is");
        panel.frame(vec![key(egui::Key::Enter)]);
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(committed.edits, vec![StringListEdit::SetText { index: 0, language: 0, text: "Line|n".to_owned() }]);
    }

    #[test]
    fn markup_the_game_wont_read_is_warned_about_but_still_saves() {
        let syntax = text_syntax(None, Some(GameId::Halo3));
        let shown = "<color>x</color> <build_number/> |kZZ";
        let analysis = strings::analyze(shown, &syntax);
        assert!(!analysis.has_errors(), "markup never stops the tool");
        let hover = markup_hover(shown, &analysis, &[]).unwrap();
        let lines: Vec<&str> = hover.lines().collect();
        assert!(lines[0].starts_with("⚠ <color>:") && lines[0].contains("as written"), "{hover}");
        assert!(lines[1].starts_with("⚠ |k:"), "{hover}");
        assert!(lines.iter().any(|line| line.starts_with("<build_number/>: filled in by the game")), "{hover}");
        // And it commits.
        let mut panel = Panel::new(&["x"]);
        panel.frame(Vec::new());
        panel.focus_first_text(1);
        panel.frame(Vec::new());
        panel.frame(vec![text(" <color>")]);
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert!(!committed.pink);
        assert_eq!(committed.edits, vec![StringListEdit::SetText { index: 0, language: 0, text: "x <color>".to_owned() }]);
    }

    #[test]
    fn halo4_bindings_without_a_substitution_pair_are_warned_about() {
        let path = locate_definitions_root().join("halo4_mcc/multilingual_unicode_string_list.json");
        let mut tag = TagFile::new(path).unwrap();
        let mut texts = vec![None; 17];
        texts[0] = Some("{cui:listing.prop_item_count}".to_owned());
        let entry = StringEntry { id: Some("a".to_owned()), texts };
        strings::apply_string_list_edits(&mut tag, Some(GameId::Halo4), &[StringListEdit::Insert { at: 0, entries: vec![entry] }]).unwrap();
        let list = strings::read_string_list(&tag).unwrap();
        let syntax = text_syntax(None, Some(GameId::Halo4));
        let shown = "{cui:listing.prop_item_count} {cui:TEMP GAMERTAG}";
        let analysis = strings::analyze(shown, &syntax);
        let unpaired = unpaired_bindings(shown, &analysis, &list, Some(GameId::Halo4));
        assert_eq!(unpaired.iter().map(|range| &shown[range.clone()]).collect::<Vec<_>>(), ["{cui:listing.prop_item_count}"]);
        assert!(markup_hover(shown, &analysis, &unpaired).unwrap().contains("no substitution pair"));
        // Reach's pairs are rebuilt on every edit, so nothing is missing there.
        assert!(unpaired_bindings(shown, &analysis, &list, Some(GameId::HaloReach)).is_empty());
    }

    #[test]
    fn hash_keywords_complete_like_glyphs() {
        let mut panel = Panel::new(&["Killed by "]);
        let shown = open_list(&mut panel, 10, "#cause_p");
        for name in ["#cause_player", "#winning_team", "#score_to_win"] {
            assert!(shown.texts.iter().any(|text| text == name), "{name} offered: {:?}", shown.texts);
        }
        assert!(shown.texts.iter().any(|text| text == "Filter…"), "Halo 3's 17 keywords don't all fit");
        assert!(!shown.pink, "an unknown #word is plain text");
        panel.frame(vec![key(egui::Key::Enter)]);
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(
            committed.edits,
            vec![StringListEdit::SetText { index: 0, language: 0, text: "Killed by #cause_player".to_owned() }]
        );
    }

    #[test]
    fn escape_closes_the_list_and_a_doubled_ampersand_never_opens_it() {
        let mut panel = Panel::new(&["a"]);
        panel.frame(Vec::new());
        panel.focus_first_text(1);
        panel.frame(Vec::new());
        panel.frame(vec![text(" &")]);
        assert!(panel.frame(Vec::new()).texts.iter().any(|text| text == "&a_button"));
        let closed = panel.frame(vec![key(egui::Key::Escape)]);
        assert!(!closed.texts.iter().any(|text| text == "&a_button"), "Escape closed it");
        panel.frame(vec![text("&")]);
        let doubled = panel.frame(Vec::new());
        assert!(!doubled.texts.iter().any(|text| text == "&a_button"), "&& is a literal &");
        assert!(!doubled.pink);
    }

    #[test]
    fn text_the_tool_would_refuse_is_pink_and_never_commits() {
        let mut panel = Panel::new(&["Films"]);
        panel.frame(Vec::new());
        panel.focus_first_text(5);
        panel.frame(Vec::new());
        let typed = panel.frame(vec![text(" & Clips")]);
        assert!(typed.pink);
        let entered = panel.frame(vec![key(egui::Key::Enter)]);
        assert!(entered.edits.is_empty(), "{:?}", entered.edits);
        let fixed = panel.frame(vec![key(egui::Key::Backspace); 6].into_iter().chain([text("& Clips"), key(egui::Key::Enter)]).collect());
        assert_eq!(fixed.edits, vec![StringListEdit::SetText { index: 0, language: 0, text: "Films & Clips".to_owned() }]);
    }

    #[test]
    fn tab_also_inserts_and_focus_stays_in_the_box() {
        let mut panel = Panel::new(&[""]);
        panel.frame(Vec::new());
        panel.focus_first_text(0);
        panel.frame(Vec::new());
        panel.frame(vec![text("&dpad_u")]);
        panel.frame(Vec::new());
        let picked = panel.frame(vec![key(egui::Key::Tab)]);
        assert!(picked.texts.iter().any(|text| text == "&dpad_up"), "{:?}", picked.texts);
        assert_eq!(panel.ctx.memory(|memory| memory.focused()), panel.text_id, "Tab did not move focus");
    }

    #[test]
    fn escape_puts_back_text_the_tool_would_refuse() {
        let mut panel = Panel::new(&["Films"]);
        panel.frame(Vec::new());
        panel.focus_first_text(5);
        panel.frame(Vec::new());
        panel.frame(vec![text(" & Clips")]);
        let escaped = panel.frame(vec![key(egui::Key::Escape)]);
        let after = panel.frame(Vec::new());
        assert!(escaped.edits.is_empty() && after.edits.is_empty());
        assert!(after.texts.iter().any(|text| text == "Films"), "{:?}", after.texts);
        assert!(!after.pink);
    }

    /// Typing a space at the end of a text keeps it, even right after a tag:
    /// drafts that compare trimmed took it for the value the tag held.
    #[test]
    fn a_trailing_space_typed_at_the_end_stays() {
        let text = "WEAPONS <color argb=#55ffffff>(1 of 7)</color>";
        let mut panel = Panel::new(&[text]);
        panel.frame(Vec::new());
        panel.focus_first_text(text.chars().count());
        panel.frame(Vec::new());
        panel.frame(vec![egui::Event::Text(" ".to_owned())]);
        let after = panel.frame(Vec::new());
        assert!(after.texts.iter().any(|shown| shown == &format!("{text} ")), "{:?}", after.texts);
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(committed.edits, vec![StringListEdit::SetText { index: 0, language: 0, text: format!("{text} ") }]);
    }

    fn click_at(panel: &mut Panel, at: egui::Pos2) -> Frame {
        let from = at - Vec2::new(20.0, 20.0);
        for step in 1..=3 {
            panel.frame(vec![egui::Event::PointerMoved(from + (at - from) * step as f32 / 3.0)]);
        }
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        panel.frame(vec![press(true)]);
        panel.frame(vec![press(false)])
    }

    #[test]
    fn a_colour_tag_has_a_swatch_that_opens_the_colour_picker() {
        let text = "W <color argb=#80ff0000>(1 of 7)</color>";
        let mut panel = Panel::new(&[text]);
        let frame = panel.frame(Vec::new());
        let red = Color32::from_rgba_unmultiplied(0xFF, 0, 0, 0x80);
        let swatch = frame.rects.iter().find(|(_, fill)| *fill == red).map(|(rect, _)| *rect).expect("a swatch in the tag's colour");
        assert!((swatch.width() - SWATCH_SIZE).abs() < 0.01);
        assert!(frame.texts.iter().any(|shown| shown == text), "the text itself is unchanged: {:?}", frame.texts);
        let clicked = click_at(&mut panel, swatch.center());
        let picker = clicked.colour_picker.expect("the click opens the colour picker");
        assert_eq!(picker.color32(), red, "on the tag's colour");
        // OK writes the digits back, as eight upper-case hex digits.
        let Some(ColorPopupResult::StringListOp { op, .. }) = picker.string_list_colour_ok() else { panic!() };
        assert_eq!(op.edit, StringListEdit::SetText { index: 0, language: 0, text: text.replace("80ff0000", "80FF0000") });
    }

    #[test]
    fn picking_a_colour_first_saves_what_was_typed() {
        let text = "<color argb=#FF00FF00>go";
        let mut panel = Panel::new(&[text]);
        panel.frame(Vec::new());
        panel.focus_first_text(text.chars().count());
        panel.frame(Vec::new());
        let typed = panel.frame(vec![egui::Event::Text("!".to_owned())]);
        let green = Color32::from_rgb(0, 0xFF, 0);
        let swatch = typed.rects.iter().find(|(_, fill)| *fill == green).map(|(rect, _)| *rect).unwrap();
        let mut edits = Vec::new();
        let from = swatch.center() - Vec2::new(20.0, 20.0);
        for step in 1..=3 {
            edits.extend(panel.frame(vec![egui::Event::PointerMoved(from + (swatch.center() - from) * step as f32 / 3.0)]).edits);
        }
        let press = |pressed| egui::Event::PointerButton { pos: swatch.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        edits.extend(panel.frame(vec![press(true)]).edits);
        let released = panel.frame(vec![press(false)]);
        edits.extend(released.edits);
        assert_eq!(edits, vec![StringListEdit::SetText { index: 0, language: 0, text: format!("{text}!") }]);
        let picker = released.colour_picker.expect("opens");
        let Some(ColorPopupResult::StringListOp { op, .. }) = picker.string_list_colour_ok() else { panic!() };
        assert_eq!(op.edit, StringListEdit::SetText { index: 0, language: 0, text: "<color argb=#FF00FF00>go!".to_owned() });
    }

    #[test]
    fn a_read_only_list_takes_no_typing() {
        let mut panel = Panel::new(&["Fixed"]);
        panel.editable = false;
        panel.frame(Vec::new());
        panel.focus_first_text(5);
        panel.frame(Vec::new());
        let frame = panel.frame(vec![text(" &x"), key(egui::Key::Enter)]);
        assert!(frame.edits.is_empty());
        assert!(frame.texts.iter().any(|text| text == "Fixed"));
    }

    fn list_of(texts: &[(&str, &str)]) -> StringList {
        let mut tag = string_list_tag(&[]);
        let entries = texts
            .iter()
            .map(|(id, text)| {
                let mut texts = vec![None; 12];
                texts[0] = Some((*text).to_owned());
                StringEntry { id: Some((*id).to_owned()), texts }
            })
            .collect();
        strings::apply_string_list_edits(&mut tag, None, &[StringListEdit::Insert { at: 0, entries }]).unwrap();
        strings::read_string_list(&tag).unwrap()
    }

    #[test]
    fn pasting_strings_lines_sets_known_ids_and_adds_the_rest() {
        let list = list_of(&[("a", "A"), ("b", "B")]);
        let syntax = text_syntax(None, Some(GameId::Halo3));
        crate::app::editor::fields::with_test_edit_context(|edit| {
            let mut view = StringListView { language: 1, selected: BTreeSet::from([0]), ..StringListView::default() };
            paste(&list, &syntax, edit, &mut view, "[Strings]\r\nb = \"Bee &a_button\"\r\nnew = \"New\"\r\n");
            let mut texts = vec![None; 12];
            texts[1] = Some("New".to_owned());
            assert_eq!(
                edit.string_list_ops.iter().map(|op| op.edit.clone()).collect::<Vec<_>>(),
                vec![
                    StringListEdit::SetText { index: 1, language: 1, text: "Bee \u{E100}".to_owned() },
                    StringListEdit::Insert { at: 1, entries: vec![StringEntry { id: Some("new".to_owned()), texts }] },
                ]
            );
            assert_eq!(view.selected, BTreeSet::from([1]));
        });
    }

    #[test]
    fn copying_writes_strings_lines_the_tool_reads() {
        let list = list_of(&[("a", "Say \"hi\" & \u{E100}"), ("b", "B")]);
        let syntax = text_syntax(None, Some(GameId::Halo3));
        let ctx = egui::Context::default();
        let output = crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            crate::app::editor::fields::with_test_edit_context(|edit| {
                let mut view = StringListView { selected: BTreeSet::from([0, 1]), ..StringListView::default() };
                apply_actions(ui, &list, &syntax, false, edit, &mut view, ToolbarActions { copy: true, ..ToolbarActions::default() });
            });
        });
        let copied = crate::app::copied_text(&output.platform_output);
        assert_eq!(copied, "a = \"Say \\\"hi\\\" && &a_button\"\r\nb = \"B\"\r\n");
        assert_eq!(strings::parse_strings_file(&copied, &syntax).unwrap()[0].1, "Say \"hi\" & \u{E100}");
    }

    #[test]
    fn removing_asks_first_unless_in_expert_mode() {
        let list = list_of(&[("a", "A"), ("b", "B")]);
        let syntax = text_syntax(None, Some(GameId::Halo3));
        let ctx = egui::Context::default();
        crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            crate::app::editor::fields::with_test_edit_context(|edit| {
                let remove = || ToolbarActions { remove: true, ..ToolbarActions::default() };
                let mut view = StringListView { selected: BTreeSet::from([1]), ..StringListView::default() };
                apply_actions(ui, &list, &syntax, false, edit, &mut view, remove());
                assert!(view.confirm_remove && edit.string_list_ops.is_empty());
                apply_actions(ui, &list, &syntax, false, edit, &mut view, ToolbarActions { confirm_remove: true, ..ToolbarActions::default() });
                assert_eq!(edit.string_list_ops.iter().map(|op| op.edit.clone()).collect::<Vec<_>>(), vec![StringListEdit::Remove { indices: vec![1] }]);
                edit.string_list_ops.clear();
                let mut view = StringListView { selected: BTreeSet::from([0]), ..StringListView::default() };
                apply_actions(ui, &list, &syntax, true, edit, &mut view, remove());
                assert_eq!(edit.string_list_ops.iter().map(|op| op.edit.clone()).collect::<Vec<_>>(), vec![StringListEdit::Remove { indices: vec![0] }]);
            });
        });
    }

    #[test]
    fn duplicates_get_ids_of_their_own() {
        let list = list_of(&[("a", "A"), ("a_copy", "A2")]);
        let syntax = text_syntax(None, Some(GameId::Halo3));
        let ctx = egui::Context::default();
        crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            crate::app::editor::fields::with_test_edit_context(|edit| {
                let mut view = StringListView { selected: BTreeSet::from([0, 1]), ..StringListView::default() };
                apply_actions(ui, &list, &syntax, false, edit, &mut view, ToolbarActions { duplicate: true, ..ToolbarActions::default() });
                let StringListEdit::Insert { at, entries } = &edit.string_list_ops[0].edit else { panic!() };
                assert_eq!(*at, 2);
                let ids: Vec<_> = entries.iter().map(|entry| entry.id.clone().unwrap()).collect();
                assert_eq!(ids, ["a_copy_2", "a_copy_copy"]);
            });
        });
    }

    #[test]
    fn shift_click_selects_a_run_of_what_is_shown() {
        let mut view = StringListView::default();
        let visible = [0, 2, 5, 7];
        select(&mut view, &visible, 2, egui::Modifiers::NONE);
        select(&mut view, &visible, 7, egui::Modifiers::SHIFT);
        assert_eq!(view.selected, BTreeSet::from([2, 5, 7]));
        select(&mut view, &visible, 0, egui::Modifiers::COMMAND);
        assert_eq!(view.selected, BTreeSet::from([0, 2, 5, 7]));
    }

    #[test]
    fn duplicate_and_empty_ids_are_problems() {
        let list = list_of(&[("a", "A"), ("b", "B")]);
        assert!(id_problem(&list, 1, "a").is_some());
        assert!(id_problem(&list, 0, "a").is_none());
        assert!(id_problem(&list, 0, "").is_some());
        assert!(id_problem(&list, 0, " a").is_some());
    }
}
