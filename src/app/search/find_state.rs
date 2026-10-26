//! Find-in-tag state shared by the dialog, matcher, and field renderer.

/// Set of documents searched by the Find dialog.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::app) enum FindWithin {
    #[default]
    CurrentTag,
    OpenTags,
    AllTags,
}

impl FindWithin {
    /// User-facing scope name shown in the Find dialog.
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::CurrentTag => "Current Tag",
            Self::OpenTags => "Open Tags",
            Self::AllTags => "All Tags",
        }
    }
}

/// Which parts of a schema-backed tag participate in Find.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) struct FindLookIn {
    pub(in crate::app) field_names: bool,
    pub(in crate::app) field_values: bool,
    pub(in crate::app) blocks: bool,
}

impl Default for FindLookIn {
    fn default() -> Self {
        Self {
            field_names: true,
            field_values: true,
            blocks: false,
        }
    }
}

impl FindLookIn {
    /// Compact summary shown on the multi-select menu button.
    pub(in crate::app) fn label(self) -> &'static str {
        match (self.field_names, self.field_values, self.blocks) {
            (true, true, false) => "Fields",
            (true, true, true) => "Fields + Blocks",
            (true, false, false) => "Field Names",
            (false, true, false) => "Field Values",
            (false, false, true) => "Blocks",
            (true, false, true) => "Names + Blocks",
            (false, true, true) => "Values + Blocks",
            (false, false, false) => "Nothing",
        }
    }

    /// Whether scalar field values participate in this target mode.
    pub(in crate::app) fn includes_values(self) -> bool {
        self.field_values
    }

    /// Whether non-block field labels participate in this target mode.
    pub(in crate::app) fn includes_field_names(self) -> bool {
        self.field_names
    }

    /// Whether block and fixed-array labels participate in this target mode.
    pub(in crate::app) fn includes_blocks(self) -> bool {
        self.blocks
    }

    pub(in crate::app) fn is_empty(self) -> bool {
        !self.field_names && !self.field_values && !self.blocks
    }
}

/// Render surface on which one textual match occurs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::app) enum FindTargetKind {
    Label,
    Value,
    Block,
    Documentation,
    StringId(usize),
    StringText { index: usize, language: usize },
}

impl FindTargetKind {
    pub(in crate::app) fn string_row(self) -> Option<usize> {
        match self {
            Self::StringId(index) | Self::StringText { index, .. } => Some(index),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::app) enum StringFindLanguage {
    #[default]
    Current,
    All,
    Language(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) struct StringFindOptions {
    pub(in crate::app) selected_rows: bool,
    pub(in crate::app) ids: bool,
    pub(in crate::app) text: bool,
    pub(in crate::app) language: StringFindLanguage,
}

impl Default for StringFindOptions {
    fn default() -> Self {
        Self {
            selected_rows: false,
            ids: true,
            text: true,
            language: StringFindLanguage::Current,
        }
    }
}

/// The pane that opened string Find. The kit and pane scope prevent identical
/// tag paths in another workspace or split pane from receiving its navigation.
#[derive(Clone, Debug)]
pub(in crate::app) struct StringFindContext {
    pub(in crate::app) selected: std::collections::BTreeSet<usize>,
    pub(in crate::app) kit: crate::app::kits::KitId,
    pub(in crate::app) tag_key: String,
    pub(in crate::app) scope: String,
    pub(in crate::app) game: Option<crate::core::game::GameId>,
    pub(in crate::app) definitions_root: Option<std::path::PathBuf>,
    pub(in crate::app) language: usize,
    pub(in crate::app) languages: Vec<String>,
    pub(in crate::app) has_ids: bool,
    pub(in crate::app) editor_active: bool,
}

/// One exact substring occurrence in one indexed tag field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) struct FindOccurrence {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) field_path: String,
    pub(in crate::app) kind: FindTargetKind,
    pub(in crate::app) text: String,
    pub(in crate::app) range: std::ops::Range<usize>,
}

/// Persistent state for the modeless Ctrl+F window.
#[derive(Default)]
pub(in crate::app) struct FindDialogState {
    pub(in crate::app) replace_mode: bool,
    pub(in crate::app) replace_run: Option<ReplaceRun>,
    pub(in crate::app) replacement: String,
    pub(in crate::app) replace_status: Option<String>,
    pub(in crate::app) find_look_in: Option<FindLookIn>,
    pub(in crate::app) all_requested: bool,
    pub(in crate::app) selection_run: Option<StringSelectionRun>,
    pub(in crate::app) open: bool,
    pub(in crate::app) query: String,
    pub(in crate::app) within: FindWithin,
    pub(in crate::app) look_in: FindLookIn,
    pub(in crate::app) strings: Option<StringFindOptions>,
    pub(in crate::app) string_context: Option<StringFindContext>,
    pub(in crate::app) match_case: bool,
    pub(in crate::app) whole_word: bool,
    pub(in crate::app) filter_results: bool,
    pub(in crate::app) filter_run: Option<FindFilterRun>,
    pub(in crate::app) occurrences: Vec<FindOccurrence>,
    pub(in crate::app) active: Option<usize>,
    /// History restoration has no egui context; reveal its active row on the
    /// next refresh, through the same navigation used by Find Next.
    pub(in crate::app) reveal_restored_active: bool,
    pub(in crate::app) all_request_id: u64,
    pub(in crate::app) all_signature: Option<String>,
    pub(in crate::app) all_closed_occurrences: Vec<FindOccurrence>,
    pub(in crate::app) all_order: Vec<String>,
    pub(in crate::app) searching: bool,
    pub(in crate::app) progress: Option<(usize, usize)>,
    pub(in crate::app) unreadable: usize,
    /// Inputs `occurrences` was last computed from. Find refreshes every frame,
    /// and walking a large scenario takes far longer than a frame, so the walk
    /// runs only when this changes.
    pub(in crate::app) results_key: Option<String>,
    /// Render lookup built from `occurrences`, shared by every field widget.
    pub(in crate::app) matching_cells: std::sync::Arc<FindMatchingCells>,
}

pub(in crate::app) type FindMatchingCells =
    std::collections::HashSet<(String, String, FindTargetKind)>;

pub(in crate::app) struct FindFilterRun {
    pub(in crate::app) query: FindQuery,
    pub(in crate::app) kit: crate::app::kits::KitId,
    pub(in crate::app) selected_tag: Option<String>,
    pub(in crate::app) cells: std::sync::Arc<FindMatchingCells>,
    pub(in crate::app) context: Option<StringFindContext>,
}

/// What Find searches for, as the Find window edits it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct FindQuery {
    pub(in crate::app) text: String,
    pub(in crate::app) within: FindWithin,
    pub(in crate::app) look_in: FindLookIn,
    pub(in crate::app) match_case: bool,
    pub(in crate::app) whole_word: bool,
    pub(in crate::app) strings: Option<StringFindOptions>,
}

pub(in crate::app) struct StringSelectionRun {
    pub(in crate::app) query: FindQuery,
    pub(in crate::app) context: StringFindContext,
    pub(in crate::app) document_stamp: String,
    pub(in crate::app) document_id: u64,
    pub(in crate::app) rows: Vec<blam_tags::string_list::StringEntry>,
    pub(in crate::app) changed: bool,
}

/// Membership stays fixed until the next explicit Find. Content stamps
/// distinguish our validated replacements from other edits needing a refresh.
pub(in crate::app) struct ReplaceRun {
    pub(in crate::app) query: FindQuery,
    pub(in crate::app) kit: crate::app::kits::KitId,
    pub(in crate::app) selected_tag: Option<String>,
    pub(in crate::app) documents: Vec<(String, u64, String, String)>,
    pub(in crate::app) matching_cells: std::sync::Arc<FindMatchingCells>,
    pub(in crate::app) changed: bool,
}

impl FindDialogState {
    pub(in crate::app) fn explicit_find(&self) -> bool {
        self.replace_mode || self.within == FindWithin::AllTags
            || self.strings.as_ref().is_some_and(|options| options.selected_rows)
    }

    pub(in crate::app) fn replace_needs_refresh(&self) -> bool {
        self.replace_run.as_ref().is_none_or(|run| run.changed || run.query != self.query())
            || self.strings.as_ref().is_some_and(|options| {
                self.selection_run.as_ref().is_none_or(|run| self.string_context.as_ref().is_none_or(|context|
                    context.kit != run.context.kit || context.tag_key != run.context.tag_key || context.scope != run.context.scope
                    || (options.selected_rows && context.selected != run.context.selected)
                    || (options.language == StringFindLanguage::Current && context.language != run.context.language)))
            })
    }
    pub(in crate::app) fn selection_needs_refresh(&self) -> bool {
        self.selection_run.as_ref().is_none_or(|run| {
            run.changed || run.query != self.query() || self.string_context.as_ref().is_none_or(|context|
                context.kit != run.context.kit || context.tag_key != run.context.tag_key
                || context.scope != run.context.scope || context.selected != run.context.selected
                || (run.query.strings.as_ref().is_some_and(|options| options.language == StringFindLanguage::Current)
                    && context.language != run.context.language))
        })
    }
    /// The query as it stands.
    pub(in crate::app) fn query(&self) -> FindQuery {
        FindQuery {
            text: self.query.clone(),
            within: self.within,
            look_in: self.look_in,
            match_case: self.match_case,
            whole_word: self.whole_word,
            strings: self.strings.clone(),
        }
    }

    pub(in crate::app) fn set_query(&mut self, query: FindQuery) {
        self.query = query.text;
        self.within = query.within;
        self.look_in = query.look_in;
        self.match_case = query.match_case;
        self.whole_word = query.whole_word;
        self.strings = query.strings;
    }

    /// Currently selected occurrence, if its stored index remains valid.
    pub(in crate::app) fn active_occurrence(&self) -> Option<&FindOccurrence> {
        self.active.and_then(|index| self.occurrences.get(index))
    }

    /// Close Find and discard transient results while preserving its options.
    pub(in crate::app) fn close(&mut self) {
        self.replace_run = None;
        self.all_requested = false;
        self.selection_run = None;
        self.open = false;
        self.active = None;
        self.reveal_restored_active = false;
        self.occurrences.clear();
        self.results_key = None;
        self.matching_cells = Default::default();
        self.searching = false;
        self.progress = None;
        self.filter_results = false;
        self.filter_run = None;
    }
}

/// Render-only Find data installed in egui temporary memory each frame, behind
/// an `Arc`: every field widget fetches it, and egui hands out clones.
pub(in crate::app) struct FindRenderSnapshot {
    pub(in crate::app) query: String,
    pub(in crate::app) match_case: bool,
    pub(in crate::app) whole_word: bool,
    pub(in crate::app) active: Option<FindOccurrence>,
    pub(in crate::app) matching_cells: std::sync::Arc<FindMatchingCells>,
    pub(in crate::app) string_context: Option<StringFindContext>,
    pub(in crate::app) filter_results: bool,
}

/// Identity of the Foundation field currently being painted.
#[derive(Clone)]
pub(in crate::app) struct FindRenderCell {
    pub(in crate::app) tag_key: String,
    pub(in crate::app) field_path: String,
}
