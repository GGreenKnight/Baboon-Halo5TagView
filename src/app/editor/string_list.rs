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
use crate::app::search::{StringFindContext, FindOccurrence, FindMatchingCells, string_find_context_id, string_find_jump_id, string_find_ranges};

use egui_extras::{Column, TableBuilder};

use super::*;
use super::string_row_clipboard::{self as row_clipboard, RowAction, RowRequest, RowData, PasteMode};

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
const ROW_HEIGHT: f32 = 36.0;
const HEADER_HEIGHT: f32 = 26.0;
/// The gap the text leaves before a colour for its swatch.
const SWATCH_SPACE: f32 = 16.0;
const SWATCH_SIZE: f32 = 10.0;

/// What the Strings tab remembers between frames, per pane and tag.
#[derive(Clone, Default)]
struct StringListView {
    tab: StringListTab,
    language: usize,
    /// Selected strings, by index into the list.
    selected: BTreeSet<usize>,
    /// Where a shift-click extends the selection from.
    anchor: Option<usize>,
    /// Scroll this string into view on the next frame.
    scroll_to: Option<usize>,
    can_scroll_up: bool,
    can_scroll_down: bool,
    focus_new: Option<usize>,
    completion: CompletionView,
    wrapped_rows: Option<Arc<WrappedRows>>,
}

#[derive(Clone, PartialEq)]
struct WrapLayout {
    revision: (u64, u64, u64, u64),
    language: usize,
    widths: Vec<f32>,
    font: egui::FontId,
    pixels_per_point: f32,
}

#[derive(Clone)]
struct WrappedRows {
    layout: WrapLayout,
    heights: Vec<f32>,
}

#[derive(Clone)]
struct StringRowDrag {
    table: egui::Id,
    indices: BTreeSet<usize>,
}

#[derive(Clone)]
struct StringTableFocus {
    document: Option<row_clipboard::RowDocument>,
    pane: egui::Id,
}

pub(in crate::app) fn select_pasted_rows(ctx: &egui::Context, scope: &str, key: &str, selected: BTreeSet<usize>) {
    let id = egui::Id::new(("field_edit", scope, key, "string list view"));
    let mut view = ctx.data(|data| data.get_temp::<StringListView>(id)).unwrap_or_default();
    view.selected = selected;
    view.anchor = view.selected.first().copied();
    view.scroll_to = view.anchor;
    view.completion.close();
    ctx.data_mut(|data| data.insert_temp(id, view));
}

#[derive(Clone)]
struct StringTextMenu {
    id: egui::Id,
    text: String,
    selection: Option<egui::text::CCursorRange>,
}

#[derive(Clone)]
enum StringTextCommand {
    Cut(egui::text::CCursorRange),
    Paste(Option<egui::text::CCursorRange>),
    SelectAll(egui::text::CCursorRange),
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
    /// Find's matching cells while Filter Results is enabled.
    filter_key: Option<Arc<FindMatchingCells>>,
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

pub(in crate::app) fn tag_text_syntax(definitions_root: Option<&Path>, game: Option<GameId>, key: &str) -> TextSyntax {
    text_syntax(definitions_root, game).with_hud_messages(key.ends_with(".hud_message_text"))
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
    let jump_id = string_find_jump_id(edit.view_scope, edit.tag_key);
    let jump = ui.data_mut(|data| {
        let jump = data.get_temp::<FindOccurrence>(jump_id);
        data.remove::<FindOccurrence>(jump_id);
        jump
    });
    if let Some(hit) = &jump {
        view.tab = StringListTab::Strings;
        view.scroll_to = hit.kind.string_row();
        if let FindTargetKind::StringText { language, .. } = hit.kind { view.language = language; }
        view.completion.close();
        let row = hit.kind.string_row().unwrap();
        let (salt, key) = match hit.kind {
            FindTargetKind::StringId(_) => ("string id", format!("{}|strings|{row}|id", edit.tag_key)),
            FindTargetKind::StringText { language, .. } => ("string text", format!("{}|strings|{row}|{language}", edit.tag_key)),
            _ => unreachable!(),
        };
        let id = edit.widget_id((salt, &key));
        let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        let start = hit.text[..hit.range.start].chars().count();
        let end = hit.text[..hit.range.end].chars().count();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(start), egui::text::CCursor::new(end))));
        state.store(ui.ctx(), id);
    }
    draw_preview_panel_toggle(
        ui,
        &mut view.tab,
        StringListTab::Fields,
        StringListTab::Strings,
        "Strings Editor",
        ButtonIcon::UnicodeStringList,
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

/// Decode once per document change and rebuild visible rows when Find's
/// Filter Results match set changes.
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
    let filter_key = string_find_snapshot(ui, edit.view_scope, edit.tag_key)
        .filter(|snapshot| snapshot.filter_results && !snapshot.query.is_empty())
        .map(|snapshot| snapshot.matching_cells.clone());
    if let Some(cache) = &cached
        && cache.revision == revision
        && match (&cache.filter_key, &filter_key) {
            (None, None) => true,
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            _ => false,
        }
    {
        return Some(cache.clone());
    }
    let visible = if let Some(matches) = &filter_key {
        matches.iter().filter(|(key, _, _)| key == edit.tag_key)
            .filter_map(|(_, _, kind)| kind.string_row()).filter(|&index| index < list.entries.len())
            .collect::<BTreeSet<_>>().into_iter().collect()
    } else { (0..list.entries.len()).collect() };
    let cache = Arc::new(ListCache { revision, list, filter_key, visible: Arc::new(visible) });
    ui.data_mut(|data| data.insert_temp(cache_id, cache.clone()));
    Some(cache)
}

pub(in crate::app) fn publish_find_context(ui: &Ui, kit: KitId, edit: &FieldEditContext<'_>) {
    let Some(view) = ui.data(|data| data.get_temp::<StringListView>(edit.widget_id("string list view"))) else { return; };
    let Some(cache) = ui.data(|data| data.get_temp::<Arc<ListCache>>(edit.widget_id("string list cache"))) else { return; };
    let context = StringFindContext { selected: view.selected.clone(), kit, tag_key: edit.tag_key.to_owned(), scope: edit.view_scope.to_owned(), game: edit.game,
        definitions_root: edit.definitions_root.map(Path::to_path_buf), language: view.language,
        languages: cache.list.languages.iter().map(|language| language.name.clone()).collect(),
        has_ids: cache.list.storage.has_ids(), editor_active: view.tab == StringListTab::Strings };
    let id = string_find_context_id(kit, edit.tag_key);
    let other_pane = ui.data(|data| data.get_temp::<StringFindContext>(id)).is_some_and(|previous| previous.scope != context.scope);
    // In split views retain the last interacted pane instead of whichever
    // duplicate happened to render last.
    if !other_pane || ui.rect_contains_pointer(ui.max_rect()) {
        ui.data_mut(|data| data.insert_temp(id, context));
    }
}

fn string_find_snapshot(ui: &Ui, scope: &str, key: &str) -> Option<Arc<FindRenderSnapshot>> {
    ui.data(|data| data.get_temp::<Arc<FindRenderSnapshot>>(find_render_snapshot_id()))
        .filter(|snapshot| snapshot.string_context.as_ref().is_some_and(|context| context.tag_key == key && context.scope == scope))
}

/// What the toolbar and keys asked for this frame, applied after the table
/// so a cell's commit, made against the indices the table drew, goes first.
#[derive(Default)]
struct ToolbarActions {
    add: bool,
    insert: bool,
    insert_below: bool,
    duplicate: bool,
    remove: bool,
    copy: bool,
    cut: bool,
    paste: Option<(PasteMode, RowData)>,
}

fn draw_strings(
    ui: &mut Ui,
    cache: &ListCache,
    game: Option<GameId>,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
    view: &mut StringListView,
) {
    egui::Frame::NONE
        .stroke(Stroke::new(1.0, foundation_block_edge()))
        .corner_radius(FOUNDATION_CONTAINER_RADIUS)
        .show(ui, |ui| {
            // Table stripes extend into cell spacing; keep them inside the frame.
            ui.shrink_clip_rect(ui.max_rect());
            draw_strings_frame(ui, cache, game, expert_mode, edit, view);
        });
}

fn draw_strings_frame(
    ui: &mut Ui,
    cache: &ListCache,
    game: Option<GameId>,
    expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
    view: &mut StringListView,
) {
    let list = &cache.list;
    // Halo CE's syntax has no glyphs; its renderer still reads `|n`.
    let syntax = list.storage.text_syntax(text_syntax(edit.definitions_root, game));
    let editable = edit.editable;
    view.selected.retain(|&index| index < list.entries.len());
    let mut actions = ToolbarActions::default();

    let header_background = ui.painter().add(egui::Shape::Noop);
    let header = egui::Frame::NONE
        .inner_margin(6).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if list.languages.len() > 1 {
                let current = list.languages[view.language].name.clone();
                ui.push_id(edit.widget_id("string list language"), |ui| {
                    fixed_width_icon_text_dropdown_button(ui, ButtonIcon::Localization, &current, 120.0, |ui| {
                        for (index, language) in list.languages.iter().enumerate() {
                            if ui.selectable_value(&mut view.language, index, &language.name).clicked() {
                                ui.close();
                            }
                        }
                    });
                }).response.on_hover_text("Language");
            }
            icon_text_dropdown_button(ui, ButtonIcon::View, "View", |ui| {
                ui.checkbox(&mut edit.string_list_checkboxes, "Show Selection Checkboxes");
                ui.checkbox(&mut edit.string_list_wrap_text, "Wrap Text");
            });
            ui.add_space(12.0);
            let any = !view.selected.is_empty();
            ui.add_enabled_ui(editable && any, |ui| {
                icon_text_dropdown_button(ui, ButtonIcon::InsertRow, "Insert", |ui| {
                    let clipboard = row_clipboard::clipboard(ui.ctx(), list, view.language, &syntax);
                    insert_menu(ui, view.selected.len(), editable, clipboard.as_ref(), &mut actions);
                });
            });
            actions.duplicate =
                icon_text_button(ui, ButtonIcon::Duplicate, "Duplicate", editable && any)
                    .clicked();
            actions.remove =
                icon_text_button(ui, ButtonIcon::Garbage, "Delete", editable && any).clicked();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let any = !cache.visible.is_empty();
                if icon_text_button(ui, ButtonIcon::JumpUp, "Scroll to Top", any && view.can_scroll_up).clicked() {
                    view.scroll_to = cache.visible.first().copied();
                }
            });
        });
    });
    paint_frame_band(ui, header_background, header.response.rect,
        egui::CornerRadius::same(FOUNDATION_CONTAINER_RADIUS as u8),
        Stroke::new(1.0, foundation_block_edge()), foundation_block_bar(), true);
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

    // Reserve the footer before sizing the scrolling body; short tables still
    // shrink to their rows, with the footer immediately underneath.
    let footer_height = BUTTON_HEIGHT + 12.0;
    let body_height = (ui.available_height() - footer_height - HEADER_HEIGHT - ui.spacing().item_spacing.y * 2.0).max(0.0);
    let _table_rect = ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
        ui.cursor().min, Vec2::new(ui.available_width(), body_height + HEADER_HEIGHT),
    )), |ui| draw_table(ui, cache, &syntax, editable, edit, view, &mut actions)).inner;

    let footer_background = ui.painter().add(egui::Shape::Noop);
    let footer = egui::Frame::NONE
        .inner_margin(6).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_height(BUTTON_HEIGHT);
                edit.string_list_help_requested |= icon_button(
                    ui, ButtonIcon::About, "Open Strings Editor documentation", true, text_dark(),
                ).clicked();
                let shown = cache.visible.len();
                let count = list.entries.len();
                let noun = if count == 1 { "string" } else { "strings" };
                let total = if shown == count { format!("{count} {noun}") } else { format!("{shown} of {count} {noun}") };
                let hidden = view.selected.iter().filter(|index| cache.visible.binary_search(index).is_err()).count();
                let hidden = if hidden > 0 { format!(" ({hidden} hidden)") } else { String::new() };
                let mut counts = egui::text::LayoutJob::default();
                RichText::new(format!("{total} · ")).color(subtle_dark())
                    .append_to(&mut counts, ui.style(), egui::FontSelection::Default, egui::Align::Center);
                RichText::new(format!("{} selected{hidden}", view.selected.len())).color(text_dark())
                    .append_to(&mut counts, ui.style(), egui::FontSelection::Default, egui::Align::Center);
                if let Some(active) = row_clipboard::active(ui.ctx()) {
                    RichText::new(" · ").color(subtle_dark())
                        .append_to(&mut counts, ui.style(), egui::FontSelection::Default, egui::Align::Center);
                    let mode = if active.cutting { "Pending Cut" } else { "Copied" };
                    RichText::new(format!("{} {mode}", active.data.rows.len())).color(active.border_color())
                        .append_to(&mut counts, ui.style(), egui::FontSelection::Default, egui::Align::Center);
                }
                ui.label(counts);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if icon_text_button(ui, ButtonIcon::JumpDown, "Scroll to Bottom", shown > 0 && view.can_scroll_down).clicked() {
                        view.scroll_to = cache.visible.last().copied();
                    }
                });
            });
        });

    paint_frame_band(ui, footer_background, footer.response.rect,
        egui::CornerRadius::same(FOUNDATION_CONTAINER_RADIUS as u8),
        Stroke::new(1.0, foundation_block_edge()), foundation_block_bar(), false);

    // Route row shortcuts to the active pane, regardless of pointer position.
    // A focused text field continues to own its standard text shortcuts.
    let focus_id = egui::Id::new("active string table");
    let pane = edit.widget_id("string table focus");
    if edit.string_list_has_focus {
        let previous = ui.data(|data| data.get_temp::<StringTableFocus>(focus_id));
        if previous.as_ref().is_none_or(|focus| focus.document != edit.string_row_document)
            || (ui.rect_contains_pointer(ui.min_rect()) && ui.input(|input| input.pointer.any_pressed())) {
            ui.data_mut(|data| data.insert_temp(focus_id, StringTableFocus { document: edit.string_row_document.clone(), pane }));
        }
    }
    let active_table = edit.string_list_has_focus && ui.data(|data| data.get_temp::<StringTableFocus>(focus_id)).is_some_and(|focus| focus.pane == pane);
    if active_table && ui.memory(|memory| memory.focused().is_none()) {
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
            view.selected.extend(cache.visible.iter().copied());
            view.anchor = cache.visible.first().copied();
        }
        if editable && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::D)) {
            actions.duplicate = !view.selected.is_empty();
        }
        // Clipboard reads update egui memory: release the input read lock
        // before handling them, rather than trying to upgrade it in Paste.
        let events = ui.input(|input| input.events.clone());
            for event in &events {
                match event {
                    egui::Event::Copy => actions.copy = !view.selected.is_empty(),
                    egui::Event::Cut if editable => actions.cut = !view.selected.is_empty(),
                    egui::Event::Paste(text) if editable => {
                        if let Some(data) = row_clipboard::clipboard(ui.ctx(), list, view.language, &syntax)
                            .or_else(|| row_clipboard::external(text, list, view.language, &syntax).ok()) {
                            actions.paste = Some((PasteMode::Overwrite, data));
                        }
                    }
                    egui::Event::Key { key: egui::Key::Delete, pressed: true, .. } if editable => {
                        actions.remove = !view.selected.is_empty();
                    }
                    _ => {}
                }
            }
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
    actions: &mut ToolbarActions,
) -> egui::Rect {
    let list = &cache.list;
    let visible = cache.visible.clone();
    let multilingual = list.storage.has_ids();
    let language = view.language;
    let scroll_to = view.scroll_to.take().and_then(|index| visible.iter().position(|&shown| shown == index));
    let height = (ui.available_height() - HEADER_HEIGHT).max(0.0);
    let top = ui.cursor().min;
    let mut clicked: Option<(usize, egui::Modifiers)> = None;
    let table_id = edit.widget_id("string list table");
    let painter = ui.painter().clone();
    let ctx = ui.ctx().clone();
    let drop_stroke = Stroke::new(2.0, ui.visuals().selection.stroke.color);
    let separator_stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let column_spacing = ui.spacing().item_spacing.x;
    let mut controls_edge = None;
    let mut add_row_rect = None;
    let mut header_bottom = top.y;
    let mut drop_move = None;
    let checkboxes = edit.string_list_checkboxes;
    let wrap = edit.string_list_wrap_text;
    let reorder = editable && cache.filter_key.is_none();
    let index_label_width = ui.painter().layout_no_wrap("Index".to_owned(), TextStyle::Body.resolve(ui.style()), text_dark()).size().x;
    let index_value_width = ui.painter().layout_no_wrap(list.entries.len().saturating_sub(1).to_string(), TextStyle::Monospace.resolve(ui.style()), text_dark()).size().x;
    let index_width = index_label_width.max(index_value_width).ceil() + 16.0;
    let mut table = TableBuilder::new(ui)
        .id_salt(table_id)
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(height)
        .min_scrolled_height(0.0)
        .column(Column::exact(28.0).resizable(false))
        .column(Column::exact(index_width).resizable(false));
    if checkboxes {
        table = table.column(Column::exact(38.0).resizable(false));
    }
    if multilingual {
        table = table.column(Column::initial(260.0).at_least(100.0).clip(true));
    }
    table = table.column(Column::remainder().at_least(160.0).clip(true));
    if let Some(row) = scroll_to {
        table = table.scroll_to_row(row, Some(egui::Align::Center));
    }
    let scroll = table
        .header(HEADER_HEIGHT, |mut header| {
            header.col(|_| {});
            header.col(|ui| {
                controls_edge = Some(ui.max_rect().right() + column_spacing * 0.5);
                padded_index_cell(ui, |ui| {
                ui.strong("Index");
                });
            });
            if checkboxes {
                header.col(|ui| {
                controls_edge = Some(ui.max_rect().right() + column_spacing * 0.5);
                padded_table_cell(ui, |ui| {
                let any = visible.iter().any(|index| view.selected.contains(index));
                let mut all = !visible.is_empty() && visible.iter().all(|index| view.selected.contains(index));
                let partial = any && !all;
                if ui.add_enabled(!visible.is_empty(), egui::Checkbox::without_text(&mut all).indeterminate(partial))
                    .on_hover_text("Select or deselect all visible strings").changed() {
                    if all { view.selected.extend(visible.iter().copied()); }
                    else { view.selected.retain(|index| !visible.contains(index)); }
                }
                });
                });
            }
            if multilingual {
                header.col(|ui| padded_table_cell(ui, |ui| {
                    ui.strong(list.storage.identity_label());
                }));
            }
            header.col(|ui| padded_table_cell(ui, |ui| {
                header_bottom = ui.max_rect().bottom() + column_spacing * 0.5;
                if list.storage == Storage::Multilingual {
                    ui.strong(format!("{} Text", list.languages[language].name));
                } else {
                    ui.strong(list.storage.text_label());
                }
            }));
        })
        .body(|mut body| {
            // Row overlays share the scrolling body's clip, so they cannot
            // cover the fixed header or footer as rows move past the edges.
            let mut body_clip = body.ui_mut().clip_rect();
            body_clip.min.y = body_clip.min.y.max(header_bottom + 1.0);
            let row_painter = body.ui_mut().painter().with_clip_rect(body_clip);
            let heights = if wrap {
                let layout = WrapLayout {
                    revision: cache.revision, language, widths: body.widths().to_vec(),
                    font: TextStyle::Monospace.resolve(body.ui_mut().style()),
                    pixels_per_point: ctx.pixels_per_point(),
                };
                if view.wrapped_rows.as_ref().is_none_or(|rows| rows.layout != layout) {
                    let first = 2 + usize::from(checkboxes);
                    let text_width = layout.widths[first + usize::from(multilingual)];
                    let ui = body.ui_mut();
                    let heights = list.entries.iter().map(|entry| {
                        let text = entry.texts.get(language).and_then(Option::as_deref).unwrap_or_default();
                        let shown = strings::format_shown(text, syntax);
                        let mut job = highlighted_job(&shown, &strings::analyze(&shown, syntax), &[], &layout.font, &TokenColors::new());
                        job.wrap.max_width = (text_width - 24.0).max(1.0);
                        let mut height = ui.fonts_mut(|fonts| fonts.layout_job(job)).size().y + 16.0;
                        if multilingual {
                            let id = entry.id.as_deref().unwrap_or_default();
                            height = height.max(ui.painter().layout(id.to_owned(), layout.font.clone(), text_dark(), (layout.widths[first] - 24.0).max(1.0)).size().y + 16.0);
                        }
                        height.max(ROW_HEIGHT)
                    }).collect();
                    view.wrapped_rows = Some(Arc::new(WrappedRows { layout, heights }));
                }
                Some(view.wrapped_rows.as_ref().unwrap().clone())
            } else { None };
            let mut height_changes = Vec::new();
            let mut draw_row = |mut row: egui_extras::TableRow<'_, '_>| {
                if row.index() == visible.len() {
                    row.col(|_| {});
                    row.col(|_| {});
                    if checkboxes { row.col(|_| {}); }
                    row.col(|ui| padded_table_cell(ui, |ui| {
                        let width = if multilingual { ui.available_width() } else { ui.available_width().min(244.0) };
                        let image = button_icon_image(ui, ButtonIcon::Add, text_dark(), BUTTON_ICON_SIZE);
                        if ui.add_sized(Vec2::new(width, BUTTON_HEIGHT), egui::Button::image_and_text(image, "Add String")).clicked() {
                            actions.add = true;
                        }
                    }));
                    if multilingual { row.col(|_| {}); }
                    add_row_rect = Some(row.response().rect);
                    if row.response().clicked() { actions.add = true; }
                    context_menu(&row.response(), |ui| {
                        let clipboard = row_clipboard::clipboard(ui.ctx(), list, language, syntax);
                        row_actions_menu(ui, view.selected.len(), editable, false, clipboard.as_ref(), actions);
                    });
                    return;
                }
                let index = visible[row.index()];
                let entry = &list.entries[index];
                let mut controls = Vec::new();
                let row_reorder = reorder && (!checkboxes || view.selected.contains(&index));
                row.set_selected(view.selected.contains(&index));
                row.col(|ui| {
                    let mut rect = ui.available_rect_before_wrap();
                    rect.min.x += 8.0;
                    rect.max.x -= 2.0;
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    let (rect, handle) = ui.allocate_exact_size(Vec2::new(18.0, 30.0), if row_reorder { Sense::click_and_drag() } else { Sense::click() });
                    controls.push(handle.clone());
                    if !checkboxes && handle.clicked() { clicked = Some((index, ui.input(|input| input.modifiers))); }
                    if row_reorder {
                        let handle = handle.on_hover_text("Drag to reorder string in every language").on_hover_cursor(egui::CursorIcon::Grab);
                        if handle.drag_started() {
                            select_context_row(view, index);
                        }
                        handle.dnd_set_drag_payload(StringRowDrag { table: table_id, indices: view.selected.clone() });
                        if handle.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            egui::Tooltip::for_widget(&handle).at_pointer().show(|ui| {
                                ui.label(format!("Move {} strings", view.selected.len()));
                            });
                        }
                    } else if checkboxes && !view.selected.contains(&index) {
                        handle.on_hover_text("Select this row's checkbox before dragging");
                    } else if cache.filter_key.is_some() {
                        handle.on_hover_text("Show the full list to reorder strings");
                    }
                    for x in [-2.5, 2.5] {
                        for y in [-6.0, 0.0, 6.0] {
                            ui.painter().circle_filled(rect.center() + Vec2::new(x, y), 1.2, subtle_dark());
                        }
                    }
                    });
                });
                row.col(|ui| padded_index_cell(ui, |ui| {
                    let response = ui.add(egui::Label::new(RichText::new(index.to_string()).monospace()).sense(Sense::click()));
                    controls.push(response.clone());
                    if !checkboxes && response.clicked() {
                        clicked = Some((index, ui.input(|input| input.modifiers)));
                    }
                }));
                if checkboxes { row.col(|ui| padded_table_cell(ui, |ui| {
                    let mut selected = view.selected.contains(&index);
                    let response = ui.checkbox(&mut selected, "");
                    controls.push(response.clone());
                    if response.changed() {
                        if selected { view.selected.insert(index); }
                        else { view.selected.remove(&index); }
                        view.anchor = Some(index);
                    }
                })); }
                let mut text_cells = Vec::new();
                if multilingual {
                    row.col(|ui| padded_table_cell(ui, |ui| {
                        let selecting = text_row_selection_gesture(ui, checkboxes);
                        let response = id_cell(ui, list, index, editable, !selecting, edit);
                        let response = if selecting { response.interact(Sense::click()) } else { response };
                        text_cells.push((response, "string id", format!("{}|strings|{index}|id", edit.tag_key), entry.id.clone().unwrap_or_default()));
                    }));
                }
                row.col(|ui| padded_table_cell(ui, |ui| {
                    let text = entry.texts.get(language).cloned().flatten().unwrap_or_default();
                    let shown = strings::format_shown(&text, syntax);
                    let target = TextTarget { index, language, max_units: list.max_text_units(), list };
                    let selecting = text_row_selection_gesture(ui, checkboxes);
                    let response = text_cell(ui, &shown, target, syntax, editable, !selecting, edit, &mut view.completion);
                    let response = if selecting { response.interact(Sense::click()) } else { response };
                    text_cells.push((response, "string text", format!("{}|strings|{index}|{language}", edit.tag_key), shown));
                }));
                let row_response = row.response();
                if let Some(active) = row_clipboard::active(&ctx).filter(|active| edit.string_row_document.as_ref() == Some(&active.source) && active.revision == cache.revision.1 && active.indices.contains(&index)) {
                    // The sizing pass can offer effectively unbounded table
                    // cells. Bound the path before generating individual dashes.
                    let rect = row_response.rect.intersect(row_painter.clip_rect()).intersect(ctx.content_rect()).shrink(1.0);
                    if rect.is_positive() && rect.is_finite() {
                        let offset = ctx.input(|input| (input.time * 18.0).rem_euclid(12.0) as f32);
                        row_painter.extend(egui::Shape::dashed_line_with_offset(&[rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()], egui::Stroke::new(1.6, active.border_color()), &[6.0], &[6.0], offset));
                        ctx.request_repaint_after(std::time::Duration::from_millis(33));
                    }
                }
                if let Some(heights) = &heights {
                    let height = text_cells.iter().map(|(cell, ..)| cell.rect.height() + 12.0).fold(ROW_HEIGHT, f32::max);
                    if (height - heights.heights[index]).abs() > 0.5 {
                        height_changes.push((index, height));
                    }
                }
                if !checkboxes && row_response.clicked() { clicked = Some((index, ctx.input(|input| input.modifiers))); }
                let mut response = row_response;
                for control in controls { response = response.union(control); }
                let mut text_target = None;
                for (cell, salt, key, original) in text_cells {
                    let normal_press = ctx.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary)
                        && !input.modifiers.shift && !input.modifiers.ctrl && !input.modifiers.command);
                    if !checkboxes && (cell.clicked() || normal_press && cell.contains_pointer()) {
                        clicked = Some((index, ctx.input(|input| input.modifiers)));
                    }
                    if view.focus_new == Some(index) && text_target.is_none() {
                        cell.request_focus();
                        view.focus_new = None;
                    }
                    if cell.contains_pointer() {
                        let id = edit.widget_id((salt, &key));
                        let text = edit.buffers.draft_mut_exact(&key, &original).text.clone();
                        let selection = ctx.data(|data| data.get_temp::<Option<egui::text::CCursorRange>>(id.with("context selection")))
                            .unwrap_or_else(|| egui::TextEdit::load_state(&ctx, id).and_then(|state| state.cursor.char_range()));
                        text_target = Some(StringTextMenu { id, text, selection });
                    }
                    response = response.union(cell);
                }
                if !checkboxes && response.secondary_clicked() { select_context_row(view, index); }
                context_menu(&response, |ui| {
                    // Keep the text target when the pointer moves into the popup.
                    let target_id = table_id.with(("context text", index));
                    if response.secondary_clicked() {
                        ui.data_mut(|data| data.insert_temp(target_id, text_target.clone()));
                    }
                    let target = ui.data(|data| data.get_temp::<Option<StringTextMenu>>(target_id)).flatten();
                    let text_context = target.is_some();
                    if let Some(target) = target {
                        text_clipboard_menu(ui, &target, editable);
                        ui.separator();
                    }
                    let clipboard = row_clipboard::clipboard(ui.ctx(), list, language, syntax);
                    row_actions_menu(ui, view.selected.len(), editable, text_context, clipboard.as_ref(), actions);
                });
                if reorder && response.dnd_hover_payload::<StringRowDrag>().is_some_and(|source| source.table == table_id) {
                    let below = ctx.pointer_hover_pos().is_some_and(|pos| pos.y > response.rect.center().y);
                    row_painter.hline(response.rect.x_range(), if below { response.rect.bottom() } else { response.rect.top() },
                        drop_stroke);
                }
                if reorder && let Some(source) = response.dnd_release_payload::<StringRowDrag>()
                    && source.table == table_id {
                    let below = ctx.pointer_hover_pos().is_some_and(|pos| pos.y > response.rect.center().y);
                    drop_move = Some((source.indices.clone(), index + usize::from(below)));
                }
            };
            if let Some(heights) = &heights {
                let row_heights = visible.iter().map(|&index| heights.heights[index]).chain(std::iter::repeat_n(ROW_HEIGHT, usize::from(editable)));
                body.heterogeneous_rows(row_heights, &mut draw_row);
            } else {
                body.rows(ROW_HEIGHT, visible.len() + usize::from(editable), &mut draw_row);
            }
            if !height_changes.is_empty() {
                let rows = Arc::make_mut(view.wrapped_rows.as_mut().unwrap());
                for (index, height) in height_changes { rows.heights[index] = height; }
                ctx.request_repaint();
            }
        });
    painter.hline(top.x..=scroll.inner_rect.right(), header_bottom, Stroke::new(2.0, foundation_block_edge()));
    let max_offset = (scroll.content_size.y - scroll.inner_rect.height()).max(0.0);
    let can_scroll_up = scroll.state.offset.y > 0.5;
    let can_scroll_down = scroll.state.offset.y < max_offset - 0.5;
    if (view.can_scroll_up, view.can_scroll_down) != (can_scroll_up, can_scroll_down) {
        view.can_scroll_up = can_scroll_up;
        view.can_scroll_down = can_scroll_down;
        ctx.request_repaint();
    }
    // The fixed row controls form one group; only its outer edge needs a
    // divider, without creating a resize handle for those columns.
    if let Some(x) = controls_edge {
        painter.vline(x, top.y..=add_row_rect.map_or(ui.min_rect().bottom(), |rect| rect.top()), separator_stroke);
    }
    if let Some((index, modifiers)) = clicked {
        select(view, &visible, index, modifiers);
    }
    if let Some((from, before)) = drop_move {
        if edit.buffers.has_changed_prefix(&format!("{}|strings|", edit.tag_key)) {
            set_status(edit, "Finish or cancel the current string edit before reordering.".to_owned());
            return egui::Rect::from_min_max(top, ui.min_rect().max.max(top));
        }
        let order = move_string_rows(list.entries.len(), &from, before);
        if order.iter().enumerate().any(|(new, &old)| new != old) {
            view.selected = order.iter().enumerate().filter_map(|(new, old)| view.selected.contains(old).then_some(new)).collect();
            view.anchor = view.anchor.and_then(|old| order.iter().position(|&index| index == old));
            view.scroll_to = order.iter().position(|index| from.contains(index));
            view.completion.close();
            edit.block_ops.push(crate::core::document::ops::BlockOp {
                path: list.storage.rows_block().to_owned(),
                kind: crate::core::document::ops::BlockOpKind::Reorder { order },
            });
        }
    }
    egui::Rect::from_min_max(top, ui.min_rect().max.max(top))
}

/// Keep headers and row contents equally inset from the column dividers.
fn padded_table_cell<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = ui.available_rect_before_wrap().shrink2(Vec2::new(8.0, 0.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), contents).inner
}

fn padded_index_cell<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = ui.available_rect_before_wrap().shrink2(Vec2::new(8.0, 0.0));
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)),
        contents,
    ).inner
}

/// Modified field clicks select rows rather than moving the text caret. The
/// field keeps its widget ID and appearance, but temporarily senses clicks.
fn text_row_selection_gesture(ui: &Ui, checkboxes: bool) -> bool {
    let rect = ui.available_rect_before_wrap();
    let selecting = !checkboxes && ui.input(|input| {
        (input.modifiers.ctrl || input.modifiers.command || input.modifiers.shift)
            && (input.pointer.primary_down() || input.pointer.button_released(egui::PointerButton::Primary))
            && input.pointer.interact_pos().is_some_and(|pos| rect.contains(pos))
    });
    if selecting && ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary)) {
        ui.memory_mut(|memory| {
            if let Some(focused) = memory.focused() { memory.surrender_focus(focused); }
        });
    }
    selecting
}
/// Change the selection for a click on string `index`'s number.
fn select(view: &mut StringListView, visible: &[usize], index: usize, modifiers: egui::Modifiers) {
    if modifiers.shift
        && let Some(anchor) = view.anchor
        && let (Some(from), Some(to)) =
            (visible.iter().position(|&i| i == anchor), visible.iter().position(|&i| i == index))
    {
        let (from, to) = (from.min(to), from.max(to));
        if !modifiers.command && !modifiers.ctrl {
            view.selected.clear();
        }
        view.selected.extend(&visible[from..=to]);
        return;
    }
    if modifiers.command || modifiers.ctrl {
        if !view.selected.remove(&index) {
            view.selected.insert(index);
        }
    } else {
        view.selected = BTreeSet::from([index]);
    }
    view.anchor = Some(index);
}

fn select_context_row(view: &mut StringListView, index: usize) {
    if !view.selected.contains(&index) {
        view.selected = BTreeSet::from([index]);
        view.anchor = Some(index);
    }
}

/// Remove the selected entries, then insert them together at an original-list boundary.
fn move_string_rows(count: usize, selected: &BTreeSet<usize>, before: usize) -> Vec<usize> {
    let before = before.min(count);
    let at = (0..before).filter(|index| !selected.contains(index)).count();
    let mut order: Vec<_> = (0..count).filter(|index| !selected.contains(index)).collect();
    order.splice(at..at, selected.iter().copied().filter(|&index| index < count));
    order
}

fn insert_menu(ui: &mut Ui, count: usize, editable: bool, clipboard: Option<&RowData>, actions: &mut ToolbarActions) {
    let clipboard = clipboard.filter(|_| row_clipboard::insert_active(ui.ctx()));
    if let Some(data) = clipboard {
        let cutting = row_clipboard::cut(ui.ctx()).is_some_and(|active| active.token == data.token);
        let kind = if cutting { "Cut" } else { "Copied" };
        let rows = if data.rows.len() == 1 { format!("{kind} Row") } else { format!("{} {kind} Rows", data.rows.len()) };
        for (mode, position) in [(PasteMode::Above, "Above"), (PasteMode::Below, "Below")] {
            let icon = if mode == PasteMode::Below { ButtonIcon::InsertRowBelow } else { ButtonIcon::InsertRow };
            if icon_text_button(ui, icon, format!("Insert {rows} {position}"), editable).clicked() {
                actions.paste = Some((mode, data.clone()));
                ui.close();
            }
        }
        return;
    }
    let rows = if count == 1 { "Row".to_owned() } else { format!("{count} Rows") };
    if icon_text_button(ui, ButtonIcon::InsertRow, format!("Insert {rows} Above"), editable && count > 0).clicked() {
        actions.insert = true;
        ui.close();
    }
    if icon_text_button(ui, ButtonIcon::InsertRowBelow, format!("Insert {rows} Below"), editable && count > 0).clicked() {
        actions.insert_below = true;
        ui.close();
    }
}

/// Only inspect the OS clipboard while a row menu is open. Tests provide an
/// isolated clipboard so they never read or replace the user's real clipboard.
fn row_actions_menu(ui: &mut Ui, count: usize, editable: bool, text_context: bool, clipboard: Option<&RowData>, actions: &mut ToolbarActions) {
    insert_menu(ui, count, editable, clipboard, actions);
    let paste_label = if text_context { "Paste Rows" } else { "Paste" };
    let paste = if text_context {
        icon_text_button(ui, ButtonIcon::Paste, paste_label, editable && clipboard.is_some())
    } else {
        shortcut_menu_button(ui, paste_label, Some(ButtonIcon::Paste), egui::Modifiers::COMMAND, egui::Key::V, editable && clipboard.is_some())
    };
    if paste.on_disabled_hover_text(if editable { "Copy valid string rows to the clipboard first." } else { "This tag is read-only." }).clicked() {
        actions.paste = clipboard.cloned().map(|data| (PasteMode::Overwrite, data));
        ui.close();
    }
    ui.separator();
    let cut = if text_context {
        icon_text_button(ui, ButtonIcon::Cut, "Cut Rows", editable && count > 0)
    } else {
        shortcut_menu_button(ui, "Cut", Some(ButtonIcon::Cut), egui::Modifiers::COMMAND, egui::Key::X, editable && count > 0)
    };
    if cut.clicked() { actions.cut = true; ui.close(); }
    let copy = if text_context {
        icon_text_button(ui, ButtonIcon::Copy, "Copy Rows", count > 0)
    } else {
        shortcut_menu_button(ui, "Copy", Some(ButtonIcon::Copy), egui::Modifiers::COMMAND, egui::Key::C, count > 0)
    };
    if copy.clicked() {
        actions.copy = true;
        ui.close();
    }
    ui.separator();
    if shortcut_menu_button(ui, "Duplicate", Some(ButtonIcon::Duplicate), egui::Modifiers::COMMAND, egui::Key::D, editable && count > 0).clicked() {
        actions.duplicate = true;
        ui.close();
    }
    let delete = if text_context {
        icon_text_button(ui, ButtonIcon::Garbage, "Delete Rows", editable && count > 0)
    } else {
        shortcut_menu_button(ui, "Delete", Some(ButtonIcon::Garbage), egui::Modifiers::NONE, egui::Key::Delete, editable && count > 0)
    };
    if delete.clicked() {
        actions.remove = true;
        ui.close();
    }
}

fn apply_text_menu_event(ui: &Ui, id: egui::Id) {
    let event = ui.data_mut(|data| {
        let key = id.with("text menu event");
        let event = data.get_temp::<StringTextCommand>(key);
        data.remove::<StringTextCommand>(key);
        event
    });
    match event {
        Some(StringTextCommand::Cut(selection)) => {
            restore_text_menu_selection(ui, id, Some(selection));
            ui.input_mut(|input| input.events.insert(0, egui::Event::Cut));
        }
        Some(StringTextCommand::Paste(selection)) => {
            restore_text_menu_selection(ui, id, selection);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        Some(StringTextCommand::SelectAll(selection)) => restore_text_menu_selection(ui, id, Some(selection)),
        None => {}
    }
}

fn restore_text_menu_selection(ui: &Ui, id: egui::Id, selection: Option<egui::text::CCursorRange>) {
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    state.cursor.set_char_range(selection);
    state.store(ui.ctx(), id);
    ui.memory_mut(|memory| memory.request_focus(id));
}

fn remember_text_menu_selection(ui: &Ui, id: egui::Id, rect: egui::Rect) {
    if ui.rect_contains_pointer(rect) && ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Secondary)) {
        // TextEdit can lose focus and collapse its selection on the press,
        // before the context menu opens on release.
        let selection = egui::TextEdit::load_state(ui.ctx(), id).and_then(|state| state.cursor.char_range());
        ui.data_mut(|data| data.insert_temp(id.with("context selection"), selection));
    }
}

fn text_clipboard_menu(ui: &mut Ui, target: &StringTextMenu, editable: bool) {
    let StringTextMenu { id, text, selection } = target;
    let id = *id;
    let selected = selection.is_some_and(|range| !range.is_empty());
    if shortcut_menu_button(ui, "Cut Text", Some(ButtonIcon::Cut), egui::Modifiers::COMMAND, egui::Key::X, editable && selected).clicked() {
        ui.data_mut(|data| data.insert_temp(id.with("text menu event"), StringTextCommand::Cut(selection.unwrap())));
        ui.ctx().request_repaint();
        ui.close();
    }
    if shortcut_menu_button(ui, "Copy Text", Some(ButtonIcon::Copy), egui::Modifiers::COMMAND, egui::Key::C, selected).clicked() {
        ui.ctx().copy_text(selection.unwrap().slice_str(text).to_owned());
        ui.close();
    }
    if shortcut_menu_button(ui, "Paste Text", Some(ButtonIcon::Paste), egui::Modifiers::COMMAND, egui::Key::V, editable).clicked() {
        ui.data_mut(|data| data.insert_temp(id.with("text menu event"), StringTextCommand::Paste(*selection)));
        ui.ctx().request_repaint();
        ui.close();
    }
    if shortcut_menu_button(ui, "Select All Text", Some(ButtonIcon::SelectAll), egui::Modifiers::COMMAND, egui::Key::A, true).clicked() {
        ui.data_mut(|data| data.insert_temp(id.with("text menu event"), StringTextCommand::SelectAll(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count())))));
        ui.ctx().request_repaint();
        ui.close();
    }
}

fn id_cell(ui: &mut Ui, list: &StringList, index: usize, editable: bool, interactive: bool, edit: &mut FieldEditContext<'_>) -> egui::Response {
    let current = list.entries[index].id.clone().unwrap_or_default();
    let key = format!("{}|strings|{index}|id", edit.tag_key);
    let id = edit.widget_id(("string id", &key));
    apply_text_menu_event(ui, id);
    let tag_key = edit.tag_key.to_owned();
    let game = edit.game;
    let scope = edit.view_scope.to_owned();
    let syntax = list.storage.text_syntax(text_syntax(edit.definitions_root, game));
    let storage = list.storage;
    let wrap = edit.string_list_wrap_text;
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, width: f32| {
        let mut job = egui::text::LayoutJob::default();
        let color = if id_problem(list, index, buffer.as_str()).is_some() { invalid_value_text() } else { text_dark() };
        job.append(buffer.as_str(), 0.0, egui::TextFormat::simple(ui.style().text_styles[&TextStyle::Monospace].clone(), color));
        let mut job = string_find_job(ui, buffer.as_str(), job, &scope, &tag_key, FindTargetKind::StringId(index), &syntax);
        if wrap { job.wrap.max_width = width; }
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let draft = edit.buffers.draft_mut_exact(&key, &current);
    let rect = ui.available_rect_before_wrap();
    remember_text_menu_selection(ui, id, rect);
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
            .interactive(interactive)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 2)))
            .font(egui::TextStyle::Monospace)
            .layouter(&mut layouter)
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
    if !valid_id(&draft.text) || (storage == Storage::HudMessages && strings::validate_hud_name(&draft.text).is_err()) {
        status = keep_refused_draft(ui, &response, draft, &current);
    } else if editable && draft.should_commit(ui, &response) {
        edit.string_list_ops.push(StringListOp { game: edit.game, edit: StringListEdit::SetId { index, id: draft.text.clone() } });
    }
    draft.keep_commit(|| {
        DraftCommit::new(&tag_key, vec![key.clone()], move |texts| {
            let id = texts[0];
            if storage == Storage::HudMessages { strings::validate_hud_name(id)?; }
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
    response
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.trim() == id
}

/// What is wrong with `id` as string `index`'s id, if anything.
fn id_problem(list: &StringList, index: usize, id: &str) -> Option<String> {
    if list.storage == Storage::HudMessages { if let Err(error) = strings::validate_hud_name(id) { return Some(error); } }
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
    interactive: bool,
    edit: &mut FieldEditContext<'_>,
    completion: &mut CompletionView,
) -> egui::Response {
    let TextTarget { index, language, max_units, list } = target;
    let key = format!("{}|strings|{index}|{language}", edit.tag_key);
    let id = edit.widget_id(("string text", &key));
    apply_text_menu_event(ui, id);
    let filter_id = id.with("completion filter");
    let tag_key = edit.tag_key.to_owned();
    let scope = edit.view_scope.to_owned();
    let game = edit.game;
    let draft = edit.buffers.draft_mut_exact(&key, shown);
    let ours = completion.cell == key;
    let focused = ui.memory(|memory| memory.has_focus(id));
    let filter_focused = ours && ui.memory(|memory| memory.has_focus(filter_id));
    let allow_completion = interactive && !egui::Popup::is_any_open(ui.ctx())
        && !ui.input(|input| input.pointer.button_down(egui::PointerButton::Secondary)
            || input.pointer.button_released(egui::PointerButton::Secondary));
    if !allow_completion && ours { completion.close(); }

    if editable && focused {
        ui.input_mut(|input| {
            for event in &mut input.events {
                if let egui::Event::Paste(text) | egui::Event::Text(text) = event {
                    *text = shown_input(text, syntax);
                }
            }
        });
    }

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
    if editable && allow_completion && (focused || filter_focused) {
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
    remember_text_menu_selection(ui, id, rect);
    let font_id = ui.style().text_styles[&TextStyle::Monospace].clone();
    let colors = TokenColors::new();
    let wrap = edit.string_list_wrap_text;
    let mut layouter = |ui: &Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let analysis = strings::analyze(buffer.as_str(), syntax);
        let unpaired = unpaired_bindings(buffer.as_str(), &analysis, list, game);
        let mut job = highlighted_job(buffer.as_str(), &analysis, &unpaired, &font_id, &colors);
        if analysis.has_errors() || analysis.text.encode_utf16().count() > max_units {
            for section in &mut job.sections { section.format.color = invalid_value_text(); }
        }
        let mut job = string_find_job(ui, buffer.as_str(), job, &scope, &tag_key, FindTargetKind::StringText { index, language }, syntax);
        if wrap { job.wrap.max_width = wrap_width; }
        ui.fonts_mut(|fonts| fonts.layout_job(job))
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
            .interactive(interactive)
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
        response.on_hover_text(if list.storage == Storage::HudMessages { "Unknown HUD token or invalid escape. Use % followed by a button, action or custom substitution; %% represents a literal percent." } else { "The tool would refuse this text: a literal & is &&, and a glyph is &name or &U+XXXX." })
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
    if editable && allow_completion && (cell_focused || filter_has_focus) {
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
    response
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

/// Convert literal game glyphs on input while leaving markup and ordinary
/// translated text intact. Numeric notation avoids ambiguous glyph names.
pub(super) fn shown_input(text: &str, syntax: &TextSyntax) -> String {
    text.chars().map(|c| {
        let raw = c.to_string();
        if !c.is_ascii() && strings::format_shown(&raw, syntax).starts_with('&') {
            format!("&U+{:04X}", u32::from(c))
        } else {
            raw
        }
    }).collect()
}

/// The glyph or keyword being typed at byte `cursor`, unless Escape closed
/// its list or the caret follows a completed name.
fn open_completion(
    syntax: &TextSyntax,
    shown: &str,
    cursor: usize,
    completion: &mut CompletionView,
    key: &str,
) -> Option<strings::Completion> {
    // The engine's ASCII completion scanner subtracts one byte before a
    // name. Mask multibyte text with equal-length spaces so its offsets stay
    // valid, including ordinary translated text that should remain literal.
    let ascii;
    let completion_text = if shown.is_ascii() {
        shown
    } else {
        ascii = shown.bytes().map(|byte| if byte.is_ascii() { byte as char } else { ' ' }).collect::<String>();
        ascii.as_str()
    };
    let found = strings::completion_at(completion_text, cursor, syntax);
    if completion.cell == key {
        match (&found, completion.dismissed_at) {
            (Some(found), Some(at)) if found.replace.start == at => return None,
            (_, Some(_)) => completion.dismissed_at = None,
            _ => {}
        }
    }
    let mut found = found?;
    // Numeric glyph references are editor notation for a character, not a
    // glyph name. Keep name suggestions closed even with the caret inside it.
    if found.kind == strings::CompletionKind::Glyph && shown[found.replace.start..].starts_with("&U+") {
        return None;
    }
    let choices = strings::choices(syntax, found.kind);
    // A click inside an existing name gives the scanner only its prefix as
    // `typed`. Match the complete token under the caret when it is known,
    // including underscores, so its own entry is highlighted. Incomplete
    // names still use prefix matching while the user types.
    if cursor < found.replace.end && matches!(found.kind, strings::CompletionKind::Hud | strings::CompletionKind::Glyph | strings::CompletionKind::Keyword) {
        let name = &shown[found.replace.start + 1..found.replace.end];
        if choices.iter().any(|choice| choice.name.eq_ignore_ascii_case(name)) {
            found.typed = name.to_owned();
            return Some(found);
        }
    }
    let finished = choices.iter().any(|choice| choice.name.eq_ignore_ascii_case(&found.typed));
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
    code_point: Color32,
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
            escape: markup_warning_text(),
            glyph: foundation_blue(),
            code_point: good_news(),
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

fn string_find_job(
    ui: &Ui, shown: &str, base: egui::text::LayoutJob, scope: &str, key: &str,
    kind: FindTargetKind, syntax: &TextSyntax,
) -> egui::text::LayoutJob {
    let Some(snapshot) = string_find_snapshot(ui, scope, key) else { return base; };
    if !snapshot.matching_cells.contains(&(key.to_owned(), String::new(), kind)) { return base; }
    let ranges = if matches!(kind, FindTargetKind::StringId(_)) {
        find_text_ranges(shown, &snapshot.query, snapshot.match_case, snapshot.whole_word)
    } else {
        string_find_ranges(shown, &snapshot.query, syntax, snapshot.match_case, snapshot.whole_word)
    };
    let active = snapshot.active.as_ref().filter(|hit| hit.tag_key == key && hit.kind == kind && hit.text == shown).map(|hit| &hit.range);
    let mut job = egui::text::LayoutJob::default();
    for section in base.sections {
        let start = section.byte_range.start.0;
        let end = section.byte_range.end.0;
        let mut cuts = vec![start, end];
        for range in &ranges {
            if start < range.start && range.start < end { cuts.push(range.start); }
            if start < range.end && range.end < end { cuts.push(range.end); }
        }
        cuts.sort_unstable(); cuts.dedup();
        for cut in cuts.windows(2) {
            let mut format = section.format.clone();
            if let Some(range) = ranges.iter().find(|range| range.contains(&cut[0])) {
                format.background = if active == Some(range) { Color32::from_rgb(255, 168, 38) } else { Color32::from_rgb(255, 220, 70) };
                format.color = Color32::BLACK;
            }
            job.append(&shown[cut[0]..cut[1]], if cut[0] == start { section.leading_space } else { 0.0 }, format);
        }
    }
    job
}

/// Documentation examples use the same tokenizer and theme colours as fields.
pub(in crate::app) fn highlighted_example(shown: &str, font: &FontId) -> egui::text::LayoutJob {
    let syntax = text_syntax(None, Some(GameId::Halo3));
    highlighted_job(shown, &strings::analyze(shown, &syntax), &[], font, &TokenColors::new())
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
    let unsupported = unsupported_keywords(shown, analysis);
    for range in &unsupported { cuts.extend([range.start, range.end]); }
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
            Some(TokenKind::Glyph) => {
                format.color = if token.is_some_and(|token| shown[token.range.clone()].starts_with("%custom-") || shown[token.range.clone()].starts_with("%{custom-")) {
                    colors.keyword
                } else if token.is_some_and(|token| shown[token.range.clone()].starts_with("&U+")) {
                    colors.code_point
                } else {
                    colors.glyph
                };
            }
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
        if unsupported.iter().any(|range| range.contains(&start)) {
            format.color = colors.error;
            format.underline = Stroke::new(1.0, colors.error);
        }
        let swatch = analysis.markup.iter().any(|span| span.color.is_some() && span.range.start == start);
        job.append(&shown[start..end], if swatch { SWATCH_SPACE } else { 0.0 }, format);
    }
    job
}

fn unsupported_keywords(shown: &str, analysis: &strings::Analysis) -> Vec<Range<usize>> {
    shown.match_indices('#').filter_map(|(start, _)| {
        if analysis.markup.iter().any(|span| span.range.contains(&start)) { return None; }
        let length = shown[start + 1..].chars().take_while(|c| c.is_ascii_lowercase() || *c == '_').count();
        (length > 0).then_some(start..start + 1 + length)
    }).collect()
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
        ui.visuals().widgets.inactive.bg_stroke
    };
    ui.painter().rect_stroke(rect, ui.visuals().widgets.inactive.corner_radius, stroke, egui::StrokeKind::Inside);
    output
}

fn cell_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(rect.center(), Vec2::new(rect.width(), (rect.height() - 12.0).max(24.0).min(rect.height())))
}

/// Fill the shape reserved under a cell's box.
fn paint_cell_fill(ui: &Ui, background: egui::layers::ShapeIdx, rect: egui::Rect, fill: Option<Color32>) {
    let shape = egui::Shape::rect_filled(cell_rect(rect), ui.visuals().widgets.inactive.corner_radius, fill.unwrap_or(ui.visuals().text_edit_bg_color()));
    ui.painter().set(background, shape);
    if fill == Some(invalid_value_fill()) {
        ui.painter().rect_stroke(cell_rect(rect), ui.visuals().widgets.inactive.corner_radius, invalid_value_stroke(), egui::StrokeKind::Inside);
    }
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
    _ui: &Ui,
    list: &StringList,
    _syntax: &TextSyntax,
    _expert_mode: bool,
    edit: &mut FieldEditContext<'_>,
    view: &mut StringListView,
    actions: ToolbarActions,
) {
    let count = list.entries.len();
    if row_clipboard::cut(_ui.ctx()).is_some_and(|active| edit.string_row_document.as_ref() == Some(&active.source))
        && edit.buffers.has_changed_prefix(&format!("{}|strings|", edit.tag_key)) {
        row_clipboard::cancel_cut(_ui.ctx());
        set_status(edit, "Cut cancelled because the source changed. The clipboard can still be pasted as a copy.".to_owned());
    }
    let last = view.selected.last().copied();
    if actions.add {
        insert_entries(edit, view, count, vec![list.new_entry()]);
        view.focus_new = Some(count);
    }
    if actions.insert || actions.insert_below {
        let groups = insertion_groups(&view.selected, actions.insert_below);
        let mut taken: BTreeSet<String> = list.entries.iter().filter_map(|entry| entry.id.clone()).collect();
        // Descending original positions keep every insertion at the intended entry.
        for &(at, count) in groups.iter().rev() {
            edit.string_list_ops.push(StringListOp { game: edit.game,
                edit: StringListEdit::Insert { at, entries: blank_entries(list, count, &mut taken) } });
        }
        let mut added = 0;
        view.selected.clear();
        for (at, count) in groups {
            view.selected.extend(at + added..at + added + count);
            added += count;
        }
        view.anchor = view.selected.first().copied();
        view.scroll_to = view.anchor;
    }
    if actions.duplicate && let Some(last) = last {
        let mut taken: BTreeSet<String> = list.entries.iter().filter_map(|entry| entry.id.clone()).collect();
        let copies = view
            .selected
            .iter()
            .map(|&index| {
                let mut entry = list.entries[index].clone();
                entry.id = entry.id.map(|id| unused_copy_id(&id, &mut taken, list.storage));
                entry
            })
            .collect();
        insert_entries(edit, view, last + 1, copies);
    }
    if actions.remove && !view.selected.is_empty() {
        remove_selected(edit, view);
    }
    if actions.copy || actions.cut {
        edit.string_row_request = Some(RowRequest { action: RowAction::Copy { cut: actions.cut }, selected: view.selected.clone(), language: view.language, scope: edit.view_scope.to_owned() });
    }
    if let Some((mode, data)) = actions.paste {
        edit.string_row_request = Some(RowRequest { action: RowAction::Paste { mode, data }, selected: view.selected.clone(), language: view.language, scope: edit.view_scope.to_owned() });
    }

}

/// Each consecutive selected run gets its own equally sized insertion.
fn insertion_groups(selected: &BTreeSet<usize>, below: bool) -> Vec<(usize, usize)> {
    let mut groups = Vec::new();
    let mut indices = selected.iter().copied().peekable();
    while let Some(start) = indices.next() {
        let mut end = start + 1;
        while indices.peek() == Some(&end) {
            indices.next();
            end += 1;
        }
        groups.push((if below { end } else { start }, end - start));
    }
    groups
}

fn blank_entries(list: &StringList, count: usize, taken: &mut BTreeSet<String>) -> Vec<StringEntry> {
    (0..count).map(|_| {
        let mut entry = list.new_entry();
        if entry.id.is_some() {
            let id = std::iter::once("new_string".to_owned()).chain((1..).map(|n| format!("new_string_{n}")))
                .find(|id| !taken.contains(id)).expect("unbounded");
            taken.insert(id.clone());
            entry.id = Some(id);
        }
        entry
    }).collect()
}

/// `id` with `_copy` (or `_copy_2`, …) added, unused by any id in `taken`,
/// which it joins.
fn unused_copy_id(id: &str, taken: &mut BTreeSet<String>, storage: Storage) -> String {
    for number in 1.. {
        let suffix = if number == 1 { "_copy".to_owned() } else { format!("_copy_{number}") };
        let base = if storage == Storage::HudMessages { id.chars().take(31 - suffix.len()).collect::<String>() } else { id.to_owned() };
        let candidate = format!("{base}{suffix}");
        if taken.insert(candidate.clone()) { return candidate; }
    }
    unreachable!()
}

fn insert_entries(edit: &mut FieldEditContext<'_>, view: &mut StringListView, at: usize, entries: Vec<StringEntry>) {
    let count = entries.len();
    edit.string_list_ops.push(StringListOp { game: edit.game, edit: StringListEdit::Insert { at, entries } });
    view.selected = (at..at + count).collect();
    view.anchor = Some(at);
    view.scroll_to = Some(at);
}

fn remove_selected(edit: &mut FieldEditContext<'_>, view: &mut StringListView) {
    let indices: Vec<usize> = view.selected.iter().copied().collect();
    let first = indices.first().copied();
    edit.string_list_ops.push(StringListOp { game: edit.game, edit: StringListEdit::Remove { indices } });
    view.selected.clear();
    view.anchor = None;
    view.scroll_to = first;
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
        checkboxes: bool,
        wrap_text: bool,
        time: f64,
        /// The id of string 0's English text box.
        text_id: Option<egui::Id>,
    }

    struct Frame {
        copied: String,
        texts: Vec<String>,
        text_rects: Vec<(String, egui::Rect)>,
        grabber_dots: Vec<egui::Pos2>,
        row_borders: Vec<Color32>,
        row_border_clips: Vec<egui::Rect>,
        orders: Vec<Vec<usize>>,
        selected: BTreeSet<usize>,
        text_backgrounds: Vec<Color32>,
        pink: bool,
        edits: Vec<StringListEdit>,
        /// Rects painted, with their fills.
        rects: Vec<(egui::Rect, Color32)>,
        /// The colour picker a click asked for.
        colour_picker: Option<MaterialColorPopup>,
        row_request: Option<RowRequest>,
    }

    impl Panel {
        fn new(texts: &[&str]) -> Self {
            Self {
                ctx: egui::Context::default(),
                tag: string_list_tag(texts),
                sinks: EditSinks::default(),
                editable: true,
                checkboxes: false,
                wrap_text: false,
                time: 0.0,
                text_id: None,
            }
        }

        fn frame(&mut self, mut events: Vec<egui::Event>) -> Frame {
            self.time += 0.1;
            let mut edits = Vec::new();
            let mut text_id = None;
            let mut colour_picker = None;
            let mut orders = Vec::new();
            let mut selected = BTreeSet::new();
            let mut row_request = None;
            let (tag, sinks, editable, checkboxes, wrap_text) = (&self.tag, &mut self.sinks, self.editable, &mut self.checkboxes, &mut self.wrap_text);
            let modifiers = events.iter().rev().find_map(|event| match event {
                egui::Event::PointerButton { modifiers, .. } | egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            }).unwrap_or_default();
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            let input = crate::app::loose_fixture::screen(events, self.time);
            let output = crate::app::run_ui_test(&self.ctx, input, |ui| {
                let game = if tag.group().tag == strings::HUD_GROUP { GameId::HaloCe } else { GameId::Halo3 };
                let tag_key = if tag.group().tag == strings::HUD_GROUP { "strings.hud_message_text" } else { "strings.multilingual_unicode_string_list" };
                let mut edit = FieldEditContext::read_only(sinks, "test", tag_key);
                edit.root = Some(tag.root());
                edit.game = Some(game);
                edit.editable = editable;
                edit.string_list_has_focus = true;
                edit.string_list_checkboxes = *checkboxes;
                edit.string_list_wrap_text = *wrap_text;
                edit.string_row_document = Some(row_clipboard::RowDocument { kit: KitId(1), key: edit.tag_key.to_owned(), id: 1 });
                edit.buffers.begin_pass(ui.ctx().cumulative_pass_nr());
                draw_string_list_tag(ui, tag, (1, 1, 1, 1), Some(game), false, &mut edit);
                *checkboxes = edit.string_list_checkboxes;
                *wrap_text = edit.string_list_wrap_text;
                selected = ui.data(|data| data.get_temp::<StringListView>(edit.widget_id("string list view"))).unwrap().selected;
                orders.extend(edit.block_ops.drain(..).filter_map(|op| match op.kind {
                    crate::core::document::ops::BlockOpKind::Reorder { order } => Some(order),
                    _ => None,
                }));
                assert!(edit.string_list_ops.iter().all(|op| op.game == Some(game)), "each op names its game");
                edits.extend(edit.string_list_ops.drain(..).map(|op| op.edit));
                row_request = edit.string_row_request.take();
                let key = format!("{}|strings|0|0", edit.tag_key);
                text_id = Some(edit.widget_id(("string text", &key)));
                colour_picker = edit.color_request.take();
            });
            self.text_id = text_id;
            let mut texts = Vec::new();
            let mut text_rects = Vec::new();
            let mut grabber_dots = Vec::new();
            let mut row_borders = Vec::new();
            let mut row_border_clips = Vec::new();
            let mut text_backgrounds = Vec::new();
            let mut pink = false;
            let mut rects = Vec::new();
            for clipped in &output.shapes {
                match &clipped.shape {
                    egui::Shape::Text(text) => {
                        texts.push(text.galley.text().to_owned());
                        text_backgrounds.extend(text.galley.job.sections.iter().map(|section| section.format.background));
                        text_rects.push((text.galley.text().to_owned(), egui::Rect::from_min_size(text.pos, text.galley.size())));
                    }
                    egui::Shape::Rect(rect) => {
                        pink |= rect.fill == invalid_value_fill();
                        rects.push((rect.rect, rect.fill));
                    }
                    egui::Shape::Circle(circle) if (circle.radius - 1.2).abs() < 0.01 => grabber_dots.push(circle.center),
                    egui::Shape::LineSegment { stroke, .. } if (stroke.width - 1.6).abs() < 0.01 => {
                        row_borders.push(stroke.color);
                        row_border_clips.push(clipped.clip_rect);
                    }
                    _ => {}
                }
            }
            Frame { copied: crate::app::copied_text(&output.platform_output), texts, text_rects, grabber_dots, row_borders, row_border_clips, orders, selected, text_backgrounds, pink, edits, rects, colour_picker, row_request }
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
    fn hud_table_uses_native_names_and_rejects_unknown_runtime_tokens() {
        let mut panel = Panel::new(&[]);
        panel.tag = TagFile::new_classic(locate_definitions_root().join("haloce_mcc/hud_message_text.json"), blam_tags::classic::ClassicEngine::HaloCe).unwrap();
        strings::apply_string_list_edits(&mut panel.tag, Some(GameId::HaloCe), &[StringListEdit::Insert { at: 0, entries: vec![StringEntry { id: Some("pickup".into()), texts: vec![Some("Hold %action|n%custom-1".into())] }] }]).unwrap();
        let frame = panel.frame(Vec::new());
        for label in ["Index", "Name", "Text", "pickup", "Hold %action|n%custom-1"] {
            assert!(frame.texts.iter().any(|text| text == label), "{label} missing: {:?}", frame.texts);
        }
        assert!(!frame.pink);
        panel.focus_first_text("Hold %action|n%custom-1".len());
        let frame = panel.frame(vec![text(" %bogus")]);
        assert!(frame.pink, "invalid HUD tokens stay visibly editable");
        let frame = panel.frame(vec![key(egui::Key::Enter)]);
        assert!(frame.edits.is_empty());
    }

    #[test]
    fn hud_duplicate_names_fit_the_engine_field_and_remain_unique() {
        let name = "n".repeat(31);
        let mut taken = BTreeSet::from([name.clone()]);
        let first = unused_copy_id(&name, &mut taken, Storage::HudMessages);
        let second = unused_copy_id(&name, &mut taken, Storage::HudMessages);
        assert_ne!(first, second);
        strings::validate_hud_name(&first).unwrap(); strings::validate_hud_name(&second).unwrap();
    }

    #[test]
    fn glyphs_show_by_name_and_line_breaks_escaped() {
        let mut panel = Panel::new(&["Press \u{E100}\r\nto go", "News & Updates", "\u{E45B}", "\u{E453}_forward"]);
        let frame = panel.frame(Vec::new());
        // Every glyph by its name; a name followed by text that would make a
        // longer one by code point.
        for shown in ["Press &a_button\\r\\nto go", "News && Updates", "&stick_move_forward", "&U+E453_forward", "English", "4 strings · 0 selected"] {
            assert!(frame.texts.iter().any(|text| text.trim() == shown), "{shown:?} not in {:?}", frame.texts);
        }
        assert!(!frame.pink, "every text reads back");
    }

    #[test]
    fn string_find_reveals_language_highlights_and_filters_without_changing_checks() {
        let mut panel = Panel::new(&["One", "Two"]);
        strings::apply_string_list_edits(&mut panel.tag, Some(GameId::Halo3), &[StringListEdit::SetText {
            index: 1, language: 1, text: "Trouver".to_owned(),
        }]).unwrap();
        panel.frame(Vec::new());
        let key = "strings.multilingual_unicode_string_list";
        let view_id = egui::Id::new(("field_edit", "test", key, "string list view"));
        let mut view = panel.ctx.data(|data| data.get_temp::<StringListView>(view_id)).unwrap();
        view.selected = BTreeSet::from([0, 1]);
        panel.ctx.data_mut(|data| data.insert_temp(view_id, view));
        let hit = FindOccurrence { tag_key: key.to_owned(), field_path: String::new(), text: "Trouver".to_owned(), range: 0..7,
            kind: FindTargetKind::StringText { index: 1, language: 1 } };
        let snapshot = Arc::new(FindRenderSnapshot {
            query: "Trouver".to_owned(), match_case: false, whole_word: false, active: Some(hit.clone()),
            matching_cells: Arc::new([(key.to_owned(), String::new(), hit.kind)].into_iter().collect()),
            string_context: Some(StringFindContext { selected: BTreeSet::new(), kit: KitId(1), tag_key: key.to_owned(), scope: "test".to_owned(),
                game: Some(GameId::Halo3), definitions_root: None, language: 0,
                languages: vec!["English".to_owned(), "Japanese".to_owned()], has_ids: true, editor_active: true }),
            filter_results: true,
        });
        panel.ctx.data_mut(|data| {
            data.insert_temp(find_render_snapshot_id(), snapshot);
            data.insert_temp(string_find_jump_id("test", key), hit);
        });
        let found = panel.frame(Vec::new());
        assert_eq!(found.selected, BTreeSet::from([0, 1]));
        assert!(found.texts.iter().any(|text| text == "Trouver"));
        assert!(!found.texts.iter().any(|text| text == "s0"));
        assert!(found.texts.iter().any(|text| text == "1 of 2 strings · 2 selected (1 hidden)"), "{:?}", found.texts);
        assert!(found.text_backgrounds.contains(&Color32::from_rgb(255, 168, 38)));
        let view = panel.ctx.data(|data| data.get_temp::<StringListView>(view_id)).unwrap();
        assert_eq!(view.language, 1);
        panel.ctx.data_mut(|data| data.remove::<Arc<FindRenderSnapshot>>(find_render_snapshot_id()));
        let restored = panel.frame(Vec::new());
        assert!(restored.texts.iter().any(|text| text == "s0"));
        assert_eq!(restored.selected, BTreeSet::from([0, 1]));
    }

    #[test]
    fn string_row_checkboxes_accumulate_selection_and_select_all() {
        let mut panel = Panel::new(&["One", "Two", "Three"]);
        panel.checkboxes = true;
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        let checkbox_at = |frame: &Frame, label: &str| {
            let y = frame.text_rects.iter().find(|(text, _)| text == label).unwrap().1.center().y;
            frame.rects.iter().map(|(rect, _)| rect).find(|rect| {
                (rect.center().y - y).abs() < 2.0 && (12.0..=20.0).contains(&rect.width()) && (rect.width() - rect.height()).abs() < 1.0
            }).expect("checkbox on the index row").center()
        };
        let first = checkbox_at(&frame, "0");
        let third = checkbox_at(&frame, "2");
        click_at(&mut panel, first);
        let selected = click_at(&mut panel, third);
        assert_eq!(selected.selected, BTreeSet::from([0, 2]));
        let all = checkbox_at(&selected, "Index");
        assert_eq!(click_at(&mut panel, all).selected, BTreeSet::from([0, 1, 2]));
        assert!(click_at(&mut panel, all).selected.is_empty());
    }

    #[test]
    fn dragging_a_string_row_queues_a_reorder() {
        for unfinished in [false, true] {
        let mut panel = Panel::new(&["One", "Two", "Three"]);
        panel.frame(Vec::new());
        if unfinished {
            panel.focus_first_text(3);
            panel.frame(Vec::new());
            panel.frame(vec![text("&not_a_glyph")]);
        }
        let frame = panel.frame(Vec::new());
        let handle = frame.grabber_dots[0] + Vec2::new(2.5, 6.0);
        let target = frame.text_rects.iter().find(|(text, _)| text == "Three").unwrap().1.center() + Vec2::new(0.0, 12.0);
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default(),
        };
        panel.frame(vec![egui::Event::PointerMoved(handle), pointer(handle, true)]);
        panel.frame(vec![egui::Event::PointerMoved(handle + Vec2::new(0.0, 12.0))]);
        panel.frame(vec![egui::Event::PointerMoved(target)]);
        let dropped = panel.frame(vec![pointer(target, false)]);
        assert_eq!(dropped.orders, if unfinished { Vec::<Vec<usize>>::new() } else { vec![vec![1, 2, 0]] });
        }
    }

    fn set_selection(panel: &Panel, selected: &[usize]) {
        let id = egui::Id::new(("field_edit", "test", "strings.multilingual_unicode_string_list", "string list view"));
        let mut view = panel.ctx.data(|data| data.get_temp::<StringListView>(id)).unwrap();
        view.selected = selected.iter().copied().collect();
        view.anchor = selected.first().copied();
        panel.ctx.data_mut(|data| data.insert_temp(id, view));
    }

    fn right_click_at(panel: &mut Panel, at: egui::Pos2) -> Frame {
        let event = |pressed| egui::Event::PointerButton {
            pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default(),
        };
        panel.frame(vec![egui::Event::PointerMoved(at)]);
        panel.frame(vec![event(true)]);
        panel.frame(vec![event(false)]);
        panel.frame(Vec::new())
    }

    #[test]
    fn grouped_drag_moves_selected_rows_and_keeps_the_group_selected() {
        let mut panel = Panel::new(&["One", "Two", "Three", "Four", "Five"]);
        panel.frame(Vec::new());
        set_selection(&panel, &[0, 2]);
        let frame = panel.frame(Vec::new());
        let handle = frame.grabber_dots[0] + Vec2::new(2.5, 6.0);
        let target = frame.text_rects.iter().find(|(text, _)| text == "Five").unwrap().1.center() + Vec2::new(0.0, 12.0);
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default(),
        };
        panel.frame(vec![egui::Event::PointerMoved(handle), pointer(handle, true)]);
        panel.frame(vec![egui::Event::PointerMoved(handle + Vec2::new(0.0, 12.0))]);
        panel.frame(vec![egui::Event::PointerMoved(target)]);
        let dropped = panel.frame(vec![pointer(target, false)]);
        assert_eq!(dropped.orders, vec![vec![1, 3, 4, 0, 2]]);
        assert_eq!(dropped.selected, BTreeSet::from([3, 4]));
        for before in 1..=4 {
            assert_eq!(move_string_rows(5, &BTreeSet::from([1, 2, 3]), before), vec![0, 1, 2, 3, 4]);
        }
    }

    #[test]
    fn row_space_and_text_clicks_select_the_clicked_row() {
        let mut panel = Panel::new(&["One", "Two", "Three"]);
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        let index = frame.text_rects.iter().find(|(text, _)| text == "1").unwrap().1;
        let at = egui::pos2(index.left() - 18.0, index.center().y);
        assert_eq!(click_at(&mut panel, at).selected, BTreeSet::from([1]));
        let text = frame.text_rects.iter().find(|(text, _)| text == "Three").unwrap().1.center();
        assert_eq!(click_at(&mut panel, text).selected, BTreeSet::from([2]));
    }

    fn click_field_with_modifiers(panel: &mut Panel, at: egui::Pos2, modifiers: egui::Modifiers) -> Frame {
        let pointer = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        panel.frame(vec![egui::Event::PointerMoved(at), pointer(true)]);
        panel.frame(vec![pointer(false)])
    }

    #[test]
    fn field_clicks_edit_one_row_and_modified_clicks_select_without_editing() {
        let mut panel = Panel::new(&["One", "Two", "Three"]);
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        let at = |text: &str| frame.text_rects.iter().find(|(shown, _)| shown == text).unwrap().1.center();
        assert_eq!(click_at(&mut panel, at("One")).selected, BTreeSet::from([0]));
        assert_eq!(panel.ctx.memory(|memory| memory.focused()), panel.text_id);
        assert_eq!(click_field_with_modifiers(&mut panel, at("Three"), egui::Modifiers::SHIFT).selected, BTreeSet::from([0, 1, 2]));
        assert!(panel.ctx.memory(|memory| memory.focused()).is_none(), "Shift-click selects rows rather than entering a field");
        assert_eq!(click_field_with_modifiers(&mut panel, at("Two"), egui::Modifiers::CTRL).selected, BTreeSet::from([0, 2]));
        assert!(panel.ctx.memory(|memory| memory.focused()).is_none(), "Ctrl-click selects rows rather than entering a field");
        assert_eq!(click_at(&mut panel, at("Two")).selected, BTreeSet::from([1]));
        let second = egui::Id::new(("field_edit", "test", "strings.multilingual_unicode_string_list", ("string text", "strings.multilingual_unicode_string_list|strings|1|0")));
        assert_eq!(panel.ctx.memory(|memory| memory.focused()), Some(second));
        let edited = panel.frame(vec![text("!")]);
        assert!(edited.texts.iter().any(|text| text.contains('!')), "a normal field click still enters editing");
        let pressed = panel.frame(vec![egui::Event::PointerMoved(at("Three")), egui::Event::PointerButton {
            pos: at("Three"), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(pressed.selected, BTreeSet::from([2]), "select as editing starts, including a text-selection drag");
    }

    #[test]
    fn checkbox_selection_survives_field_row_and_context_clicks() {
        let mut panel = Panel::new(&["One", "Two", "Three"]);
        panel.checkboxes = true;
        panel.frame(Vec::new());
        set_selection(&panel, &[0, 2]);
        let frame = panel.frame(Vec::new());
        let text = frame.text_rects.iter().find(|(text, _)| text == "Two").unwrap().1.center();
        let expected = BTreeSet::from([0, 2]);
        assert_eq!(click_at(&mut panel, text).selected, expected);
        assert_eq!(click_field_with_modifiers(&mut panel, text, egui::Modifiers::CTRL).selected, expected);
        let index = frame.text_rects.iter().find(|(text, _)| text == "1").unwrap().1.center();
        assert_eq!(click_at(&mut panel, index).selected, expected);
        assert_eq!(right_click_at(&mut panel, text).selected, expected);
    }

    #[test]
    fn context_menu_preserves_a_group_and_includes_text_and_row_actions() {
        let mut panel = Panel::new(&["One", "Two", "\u{E100}"]);
        panel.frame(Vec::new());
        set_selection(&panel, &[0, 2]);
        let frame = panel.frame(Vec::new());
        let at = frame.text_rects.iter().find(|(text, _)| text == "&a_button").unwrap().1.center();
        let menu = right_click_at(&mut panel, at);
        assert_eq!(menu.selected, BTreeSet::from([0, 2]));
        assert!(!menu.texts.iter().any(|text| text.starts_with("Filter")), "completion must not appear behind a context menu");
        for label in ["Copy Text", "Paste Text", "Select All Text", "Insert 2 Rows Above", "Insert 2 Rows Below", "Copy Rows", "Paste Rows", "Cut Rows"] {
            assert!(menu.texts.iter().any(|text| text == label), "{label} missing from {:?}", menu.texts);
        }
        for key in [egui::Key::X, egui::Key::C, egui::Key::V, egui::Key::A] {
            let hint = shortcut_text(&panel.ctx, egui::Modifiers::COMMAND, key);
            assert_eq!(menu.texts.iter().filter(|text| **text == hint).count(), 1, "one text action owns {hint}");
        }
        // Escape closes the menu; right-clicking an unselected index targets it alone.
        panel.frame(vec![key(egui::Key::Escape)]);
        let frame = panel.frame(Vec::new());
        let at = frame.text_rects.iter().find(|(text, _)| text == "1").unwrap().1.center();
        let menu = right_click_at(&mut panel, at);
        assert_eq!(menu.selected, BTreeSet::from([1]));
        for (modifiers, key) in [(egui::Modifiers::COMMAND, egui::Key::C), (egui::Modifiers::NONE, egui::Key::Delete)] {
            assert!(menu.texts.contains(&shortcut_text(&panel.ctx, modifiers, key)));
        }
    }

    #[test]
    fn inserting_around_separate_groups_preserves_languages_and_selects_unique_new_entries() {
        for below in [false, true] {
            let ctx = egui::Context::default();
            crate::app::run_ui_test(&ctx, Default::default(), |ui| {
                let list = list_of(&[("a", "A"), ("b", "B"), ("c", "C"), ("d", "D"), ("e", "E")]);
                let mut view = StringListView { selected: BTreeSet::from([0, 1, 4]), ..Default::default() };
                let mut sinks = EditSinks::default();
                let mut edit = FieldEditContext::read_only(&mut sinks, "test", "strings");
                apply_actions(ui, &list, &TextSyntax::unknown(), true, &mut edit, &mut view,
                    ToolbarActions { insert: !below, insert_below: below, ..Default::default() });
                let mut tag = string_list_tag(&["A", "B", "C", "D", "E"]);
                strings::apply_string_list_edits(&mut tag, Some(GameId::Halo3), &[
                    StringListEdit::SetText { index: 1, language: 1, text: "Japanese B".to_owned() },
                    StringListEdit::SetText { index: 4, language: 1, text: "Japanese E".to_owned() },
                ]).unwrap();
                let edits: Vec<_> = edit.string_list_ops.drain(..).map(|op| op.edit).collect();
                strings::apply_string_list_edits(&mut tag, Some(GameId::Halo3), &edits).unwrap();
                let bytes = tag.write_to_bytes().unwrap();
                let reopened = TagFile::read_from_bytes(&bytes).unwrap();
                let result = strings::read_string_list(&reopened).unwrap();
                assert_eq!(result.entries.len(), 8);
                assert_eq!(result.damaged_offsets, 0);
                assert_eq!(result.entries.iter().filter_map(|entry| entry.id.clone()).collect::<BTreeSet<_>>().len(), 8);
                assert_eq!(view.selected, if below { BTreeSet::from([2, 3, 7]) } else { BTreeSet::from([0, 1, 6]) });
                let originals: Vec<_> = result.entries.iter().filter(|entry| entry.texts[0].is_some()).collect();
                assert_eq!(originals.iter().map(|entry| entry.texts[0].as_deref().unwrap()).collect::<Vec<_>>(), vec!["A", "B", "C", "D", "E"]);
                assert_eq!(originals[1].texts[1].as_deref(), Some("Japanese B"));
                assert_eq!(originals[4].texts[1].as_deref(), Some("Japanese E"));
            });
        }
    }

    #[test]
    fn copied_and_cut_rows_have_the_requested_dashed_borders_and_escape_restores_insert() {
        for (cutting, color) in [(false, Color32::from_rgb(71, 174, 235)), (true, Color32::from_rgb(255, 209, 56))] {
            let mut panel = Panel::new(&["One", "Two"]);
            let data = RowData { hud: false, token: "outline".to_owned(), classic: false, languages: vec!["English".to_owned()], language: 0,
                rows: vec![row_clipboard::Row { id: Some("new".to_owned()), texts: vec![Some("Copied".to_owned())] }] };
            row_clipboard::write(&panel.ctx, &data, &text_syntax(None, Some(GameId::Halo3))).unwrap();
            row_clipboard::set_active(&panel.ctx, row_clipboard::ActiveRows { cutting,
                source: row_clipboard::RowDocument { kit: KitId(1), key: "strings.multilingual_unicode_string_list".to_owned(), id: 1 },
                revision: 1, indices: BTreeSet::from([0]), token: data.token.clone(), data });
            panel.frame(Vec::new());
            let frame = panel.frame(Vec::new());
            assert!(frame.row_borders.contains(&color), "the copied/cut row must paint its dashed border");
            let footer_mode = if cutting { "Pending Cut" } else { "Copied" };
            assert!(frame.texts.iter().any(|text| text.ends_with(&format!(" · 1 {footer_mode}"))), "the footer shows the active clipboard row count");
            let header_bottom = frame.text_rects.iter().find(|(text, _)| text == "String ID").unwrap().1.bottom();
            assert!(frame.row_border_clips.iter().all(|clip| clip.top() >= header_bottom), "row outlines must be clipped below the table header");
            let at = frame.text_rects.iter().find(|(text, _)| text == "0").unwrap().1.center();
            let menu = right_click_at(&mut panel, at);
            let kind = if cutting { "Cut" } else { "Copied" };
            assert!(menu.texts.iter().any(|text| text == &format!("Insert {kind} Row Above")));
            panel.frame(vec![key(egui::Key::Escape)]);
            row_clipboard::dismiss(&panel.ctx);
            let menu = right_click_at(&mut panel, at);
            assert!(!menu.texts.iter().any(|text| text.contains("Pending Cut") || text.ends_with(" Copied")), "Escape removes the clipboard count");
            assert!(menu.texts.iter().any(|text| text == "Insert Row Above"));
            assert!(!menu.texts.iter().any(|text| text.starts_with("Insert Copied") || text.starts_with("Insert Cut")));
        }
    }

    #[test]
    fn control_a_selects_rows_without_checkboxes_but_preserves_text_selection() {
        let mut panel = Panel::new(&["One", "Two", "Three"]);
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        let row = frame.text_rects.iter().find(|(text, _)| text == "0").unwrap().1.center();
        click_at(&mut panel, row);
        let select_all = || egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        assert_eq!(panel.frame(vec![select_all()]).selected, BTreeSet::from([0, 1, 2]));
        let frame = panel.frame(Vec::new());
        let text = frame.text_rects.iter().find(|(text, _)| text == "One").unwrap().1.center();
        click_at(&mut panel, text);
        assert_eq!(panel.frame(vec![select_all()]).selected, BTreeSet::from([0]));
        assert_eq!(panel.frame(vec![egui::Event::Copy]).copied, "One");
    }

    #[test]
    fn row_shortcuts_follow_the_active_table_when_the_pointer_leaves_it() {
        let mut panel = Panel::new(&["One", "Two"]);
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        let row = frame.text_rects.iter().find(|(text, _)| text == "0").unwrap().1.center();
        click_at(&mut panel, row);
        for cut in [false, true] {
            let frame = panel.frame(vec![egui::Event::PointerMoved(egui::pos2(999.0, 799.0)), if cut { egui::Event::Cut } else { egui::Event::Copy }]);
            assert!(matches!(frame.row_request.as_ref().map(|request| &request.action), Some(RowAction::Copy { cut: actual }) if *actual == cut));
        }
        panel.ctx.data_mut(|data| data.insert_temp(egui::Id::new("test string row clipboard"), "new\tPasted".to_owned()));
        let pasted = panel.frame(vec![egui::Event::Paste("new\tPasted".to_owned())]);
        assert!(matches!(pasted.row_request.as_ref().map(|request| &request.action), Some(RowAction::Paste { .. })));
        let deleted = panel.frame(vec![key(egui::Key::Delete)]);
        assert!(deleted.edits.contains(&StringListEdit::Remove { indices: vec![0] }));
        // A focused text field owns Copy even with a row selected.
        let frame = panel.frame(Vec::new());
        let text = frame.text_rects.iter().find(|(text, _)| text == "One").unwrap().1.center();
        click_at(&mut panel, text);
        assert!(panel.frame(vec![egui::Event::Copy]).row_request.is_none());
    }

    #[test]
    fn row_paste_menu_checks_the_clipboard_and_copy_shortcut_still_works() {
        for (clipboard, editable, should_paste) in [
            (None, true, false),
            (Some("ordinary text"), true, false),
            (Some("new\tValid\nbroken row"), true, false),
            (Some("new\tAdded"), true, true),
            (Some("new\tAdded"), false, false),
        ] {
            let mut panel = Panel::new(&["One"]);
            panel.editable = editable;
            if let Some(text) = clipboard {
                panel.ctx.data_mut(|data| data.insert_temp(egui::Id::new("test string row clipboard"), text.to_owned()));
            }
            panel.frame(Vec::new());
            let frame = panel.frame(Vec::new());
            assert!(!frame.texts.iter().any(|text| text == "Copy" || text.contains("Paste strings with")), "clipboard controls no longer occupy the header");
            let at = frame.text_rects.iter().find(|(text, _)| text == "0").unwrap().1.center();
            click_at(&mut panel, at);
            let copied = panel.frame(vec![egui::Event::PointerMoved(at), egui::Event::Copy]);
            assert!(matches!(copied.row_request.as_ref().map(|request| &request.action), Some(RowAction::Copy { cut: false })), "Ctrl+C still copies selected rows");
            let menu = right_click_at(&mut panel, at);
            // Header buttons share labels with the menu, and Delete also has
            // a shortcut hint. Compare the menu's last painted label positions.
            let action_y = |label: &str| menu.text_rects.iter().rfind(|(text, _)| text == label).unwrap().1.center().y;
            for pair in ["Paste", "Cut", "Copy", "Duplicate", "Delete"].windows(2) {
                assert!(action_y(pair[0]) < action_y(pair[1]), "{} must precede {}", pair[0], pair[1]);
            }
            let has_rows = clipboard == Some("new\tAdded");
            for position in ["Above", "Below"] {
                assert_eq!(menu.texts.iter().any(|text| text == &format!("Insert Copied Row {position}")), has_rows);
                assert_eq!(menu.texts.iter().any(|text| text == &format!("Insert Row {position}")), !has_rows);
            }
            let paste = menu.text_rects.iter().find(|(text, _)| text == "Paste").unwrap().1.center();
            let result = click_at(&mut panel, paste);
            assert_eq!(result.row_request.is_some(), should_paste, "clipboard={clipboard:?}, editable={editable}");
            if should_paste {
                assert!(matches!(result.row_request.unwrap().action, RowAction::Paste { mode: PasteMode::Overwrite, data } if data.rows.len() == 1 && data.rows[0].id.as_deref() == Some("new") && data.rows[0].texts[0].as_deref() == Some("Added")));
            }
        }
    }

    #[test]
    fn duplicate_shortcut_acts_on_rows_but_not_a_focused_text_field() {
        let mut panel = Panel::new(&["One", "Two"]);
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        let row = frame.text_rects.iter().find(|(text, _)| text == "0").unwrap().1.center();
        click_at(&mut panel, row);
        let duplicate = || egui::Event::Key { key: egui::Key::D, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        let frame = panel.frame(vec![duplicate()]);
        assert!(frame.edits.iter().any(|edit| matches!(edit, StringListEdit::Insert { entries, .. } if entries.len() == 1 && entries[0].texts[0].as_deref() == Some("One"))));
        let frame = panel.frame(Vec::new());
        let text = frame.text_rects.iter().find(|(text, _)| text == "One").unwrap().1.center();
        click_at(&mut panel, text);
        assert!(panel.frame(vec![duplicate()]).edits.is_empty());
    }

    #[test]
    fn wrapping_fits_both_columns_grows_rows_and_preserves_the_strings() {
        let text = "A long string with spaces and 日本語 text. ".repeat(12);
        let id = "a_long_identifier_without_spaces_".repeat(9);
        let mut panel = Panel::new(&[&text, "Next row"]);
        strings::apply_string_list_edits(&mut panel.tag, Some(GameId::Halo3), &[
            StringListEdit::SetId { index: 0, id: id.clone() },
        ]).unwrap();
        panel.frame(Vec::new());
        let flat = panel.frame(Vec::new());
        let rect = |frame: &Frame, value: &str| frame.text_rects.iter().find(|(text, _)| text == value).unwrap().1;
        let next_y = rect(&flat, "Next row").top();
        panel.wrap_text = true;
        for _ in 0..3 { panel.frame(Vec::new()); }
        let wrapped = panel.frame(Vec::new());
        for value in [&text, &id] {
            assert!(rect(&wrapped, value).height() > rect(&flat, value).height() * 2.0, "{value} must wrap");
        }
        assert!(rect(&wrapped, "Next row").top() > next_y + ROW_HEIGHT);
        assert!(rect(&wrapped, "Next row").top() > rect(&wrapped, &text).bottom());
        assert!(wrapped.edits.is_empty(), "wrapping never inserts line breaks into the tag");
        panel.focus_first_text(text.chars().count());
        let added = " More words to expand the row.".repeat(20);
        panel.frame(vec![egui::Event::Text(added.clone())]);
        for _ in 0..2 { panel.frame(Vec::new()); }
        let growing = panel.frame(Vec::new());
        assert!(rect(&growing, &(text.clone() + &added)).height() > rect(&wrapped, &text).height());
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert!(committed.edits.iter().any(|edit| matches!(edit, StringListEdit::SetText { text: value, .. } if value == &(text.clone() + &added))), "Enter must still commit without adding a newline");
        panel.wrap_text = false;
        let flat_again = panel.frame(Vec::new());
        assert!((rect(&flat_again, "Next row").top() - next_y).abs() < 1.0);
    }

    #[test]
    fn bottom_placeholder_adds_one_entry_to_an_empty_list() {
        let mut panel = Panel::new(&[]);
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        let at = frame.text_rects.iter().find(|(text, _)| text == "Add String").unwrap().1.center();
        let added = click_at(&mut panel, at);
        assert!(matches!(added.edits.as_slice(), [StringListEdit::Insert { at: 0, entries }] if entries.len() == 1));
        strings::apply_string_list_edits(&mut panel.tag, Some(GameId::Halo3), &added.edits).unwrap();
        let cache_id = egui::Id::new(("field_edit", "test", "strings.multilingual_unicode_string_list", "string list cache"));
        panel.ctx.data_mut(|data| data.remove::<Arc<ListCache>>(cache_id));
        panel.frame(Vec::new());
        let key = "strings.multilingual_unicode_string_list|strings|0|id";
        let id = egui::Id::new(("field_edit", "test", "strings.multilingual_unicode_string_list", ("string id", key)));
        assert_eq!(panel.ctx.memory(|memory| memory.focused()), Some(id));
    }

    #[test]
    fn view_menu_enables_checkboxes_and_text_context_copy_uses_the_text_selection() {
        let mut panel = Panel::new(&["One", "Two"]);
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        assert!(!panel.checkboxes);
        let at = frame.text_rects.iter().find(|(text, _)| text.trim() == "View").unwrap().1.center();
        click_at(&mut panel, at);
        let menu = panel.frame(Vec::new());
        let at = menu.text_rects.iter().find(|(text, _)| text == "Show Selection Checkboxes").unwrap().1.center();
        click_at(&mut panel, at);
        assert!(panel.checkboxes);
        panel.frame(vec![key(egui::Key::Escape)]);
        panel.frame(Vec::new());
        panel.focus_first_text(3);
        panel.frame(Vec::new());
        let frame = panel.frame(Vec::new());
        let at = frame.text_rects.iter().find(|(text, _)| text == "One").unwrap().1.center();
        let menu = right_click_at(&mut panel, at);
        let select_all = menu.text_rects.iter().find(|(text, _)| text == "Select All Text").unwrap().1.center();
        click_at(&mut panel, select_all);
        panel.frame(Vec::new());
        let range = egui::TextEdit::load_state(&panel.ctx, panel.text_id.unwrap()).unwrap().cursor.char_range().unwrap();
        assert_eq!(range.slice_str("One"), "One", "Select All Text restores field focus and selection after closing the menu");
        let menu = right_click_at(&mut panel, at);
        let table_id = egui::Id::new(("field_edit", "test", "strings.multilingual_unicode_string_list", "string list table"));
        let target = panel.ctx.data(|data| data.get_temp::<Option<StringTextMenu>>(table_id.with(("context text", 0usize)))).flatten().unwrap();
        assert_eq!(target.selection.unwrap().slice_str("One"), "One", "right-click preserves the selection for clipboard commands");
        let copy = menu.text_rects.iter().find(|(text, _)| text == "Copy Text").unwrap().1.center();
        assert_eq!(click_at(&mut panel, copy).copied, "One");
    }

    #[test]
    fn header_and_footer_buttons_scroll_to_both_ends_of_a_long_list() {
        let edges = |panel: &Panel| {
            let id = egui::Id::new(("field_edit", "test", "strings.multilingual_unicode_string_list", "string list view"));
            let view = panel.ctx.data(|data| data.get_temp::<StringListView>(id)).unwrap();
            (view.can_scroll_up, view.can_scroll_down)
        };
        let texts: Vec<String> = (0..200).map(|index| format!("String {index}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut panel = Panel::new(&refs);
        panel.frame(Vec::new());
        let first = panel.frame(Vec::new());
        assert!(first.texts.iter().any(|text| text == "String 0"));
        assert!(!first.texts.iter().any(|text| text == "String 199"));
        assert_eq!(edges(&panel), (false, true));
        let bottom = first.text_rects.iter().find(|(text, _)| text == "Scroll to Bottom").unwrap().1.center();
        let top_y = first.text_rects.iter().find(|(text, _)| text == "Scroll to Top").unwrap().1.center().y;
        let count_y = first.text_rects.iter().find(|(text, _)| text == "200 strings · 0 selected").unwrap().1.center().y;
        assert!(bottom.y > top_y + 100.0);
        assert!((bottom.y - count_y).abs() < 2.0, "footer count at {count_y}, button at {}", bottom.y);
        click_at(&mut panel, bottom);
        // Allow the scroll animation to reach its target.
        for _ in 0..10 { panel.frame(Vec::new()); }
        let last = panel.frame(Vec::new());
        assert!(last.texts.iter().any(|text| text == "String 199"), "{:?}", last.texts);
        assert_eq!(edges(&panel), (true, false));
        let top = last.text_rects.iter().find(|(text, _)| text == "Scroll to Top").unwrap().1.center();
        click_at(&mut panel, top);
        for _ in 0..10 { panel.frame(Vec::new()); }
        let first = panel.frame(Vec::new());
        assert!(first.texts.iter().any(|text| text == "String 0"), "{:?}", first.texts);
        assert!(!first.texts.iter().any(|text| text == "String 199"));
        assert_eq!(edges(&panel), (false, true));
        let mut short = Panel::new(&["One"]);
        short.frame(Vec::new());
        let short_frame = short.frame(Vec::new());
        let short_bottom = short_frame.text_rects.iter().find(|(text, _)| text == "Scroll to Bottom").unwrap().1.center();
        assert!(short_bottom.y < 300.0, "short-list footer follows the rows");
        assert_eq!(edges(&short), (false, false), "a list that fits needs neither scroll button");
    }

    #[test]
    fn unicode_text_keeps_its_characters_and_completion_offsets() {
        let syntax = text_syntax(Some(&locate_definitions_root()), Some(GameId::Halo3));
        let text = "é日本語🙂 &a_b";
        assert_eq!(shown_input(text, &syntax), text);
        assert_eq!(shown_input("\u{E100} &a_button", &syntax), "&U+E100 &a_button");
        for cursor in text.char_indices().map(|(at, _)| at).chain([text.len()]) {
            let _ = open_completion(&syntax, text, cursor, &mut CompletionView::default(), "cell");
        }
        let found = open_completion(&syntax, text, text.len(), &mut CompletionView::default(), "cell").unwrap();
        assert_eq!(&text[found.replace], "&a_b");
    }

    #[test]
    fn pasting_a_unicode_glyph_converts_it_without_crashing() {
        let mut panel = Panel::new(&["Press "]);
        panel.frame(Vec::new());
        panel.focus_first_text(6);
        panel.frame(Vec::new());
        panel.frame(vec![egui::Event::Paste("\u{E156}".to_owned())]);
        let after = panel.frame(Vec::new());
        assert!(after.texts.iter().any(|text| text == "Press &U+E156"), "{:?}", after.texts);
        let committed = panel.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(committed.edits, vec![StringListEdit::SetText { index: 0, language: 0, text: "Press \u{E156}".to_owned() }]);
    }

    #[test]
    fn unknown_keywords_are_red_while_supported_keywords_and_unicode_keep_their_meaning() {
        let syntax = text_syntax(None, Some(GameId::Halo3));
        let shown = "#cause_player #not_a_supported_keyword &U+E156";
        let analysis = strings::analyze(shown, &syntax);
        let colors = TokenColors::new();
        let job = highlighted_job(shown, &analysis, &[], &egui::FontId::default(), &colors);
        let color_at = |at| job.sections.iter().find(|section| section.byte_range.contains(&egui::text::ByteIndex(at))).unwrap().format.color;
        assert_ne!(color_at(0), colors.error);
        assert_eq!(color_at(shown.find("#not").unwrap()), colors.error);
        assert_eq!(color_at(shown.find("&U+").unwrap()), colors.code_point);
        assert_eq!(strings::parse_shown("&U+E156", &syntax).unwrap(), "\u{E156}");
    }

    #[test]
    fn numeric_unicode_references_are_green_without_name_suggestions() {
        let syntax = text_syntax(Some(&locate_definitions_root()), Some(GameId::Halo3));
        let shown = "&U+E999 &a_button";
        let analysis = strings::analyze(shown, &syntax);
        let colors = TokenColors::new();
        let job = highlighted_job(shown, &analysis, &[], &egui::FontId::default(), &colors);
        let color_at = |at| job.sections.iter().find(|section| section.byte_range.contains(&egui::text::ByteIndex(at))).unwrap().format.color;
        assert_eq!(color_at(0), good_news());
        assert_eq!(color_at(8), foundation_blue());
        assert_ne!(color_at(0), color_at(8));
        for cursor in 1..=7 {
            assert_eq!(open_completion(&syntax, shown, cursor, &mut CompletionView::default(), "cell"), None, "cursor {cursor}");
        }
        assert!(open_completion(&syntax, "&a_b", 4, &mut CompletionView::default(), "cell").is_some());
        assert_eq!(strings::parse_shown("&U+E999", &syntax).unwrap(), "\u{E999}");
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
    fn clicking_inside_an_existing_shortcut_matches_its_whole_name() {
        let syntax = text_syntax(Some(&locate_definitions_root()), Some(GameId::Halo2));
        let shown = "日本語 &button_verb1 to pick up";
        let start = shown.find('&').unwrap();
        let end = start + "&button_verb1".len();
        for cursor in start + 1..end {
            let open = open_completion(&syntax, shown, cursor, &mut CompletionView::default(), "cell").unwrap();
            let choices = filtered_choices(&syntax, open.kind, "");
            let best = strings::best_match(choices.iter().map(|choice| choice.name.as_str()), &open.typed).unwrap();
            assert_eq!(choices[best].name, "button_verb1", "caret byte {cursor} matched {:?}", open.typed);
            assert_eq!(open.replace, start..end);
        }
        // Completing a name by typing it still closes the suggestions.
        assert!(open_completion(&syntax, shown, end, &mut CompletionView::default(), "cell").is_none());
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
    fn copying_requests_whole_rows_instead_of_language_only_tool_text() {
        let list = list_of(&[("a", "Say \"hi\" & \u{E100}"), ("b", "B")]);
        let syntax = text_syntax(None, Some(GameId::Halo3));
        let ctx = egui::Context::default();
        crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            crate::app::editor::fields::with_test_edit_context(|edit| {
                let mut view = StringListView { selected: BTreeSet::from([0, 1]), ..StringListView::default() };
                apply_actions(ui, &list, &syntax, false, edit, &mut view, ToolbarActions { copy: true, ..ToolbarActions::default() });
                let request = edit.string_row_request.take().unwrap();
                assert_eq!(request.selected, BTreeSet::from([0, 1]));
                assert!(matches!(request.action, RowAction::Copy { cut: false }));
            });
        });
    }

    #[test]
    fn removing_is_immediate_in_every_mode() {
        let list = list_of(&[("a", "A"), ("b", "B")]);
        let syntax = text_syntax(None, Some(GameId::Halo3));
        let ctx = egui::Context::default();
        crate::app::run_ui_test(&ctx, egui::RawInput::default(), |ui| {
            crate::app::editor::fields::with_test_edit_context(|edit| {
                let remove = || ToolbarActions { remove: true, ..ToolbarActions::default() };
                let mut view = StringListView { selected: BTreeSet::from([1]), ..StringListView::default() };
                apply_actions(ui, &list, &syntax, false, edit, &mut view, remove());
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
