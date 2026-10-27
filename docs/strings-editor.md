Open a `multilingual_unicode_string_list`, or a Halo CE `unicode_string_list` or `hud_message_text`, and choose **Strings Editor**. Each row shows its **Index**, **String ID** (multilingual lists), and the selected language's **Text**. *Index is the zero-based storage position.* **Tag Fields** shows the raw field tree.

## Reading string text

The editor shows the notation accepted by the game's import tool. These Halo 3 examples use the **same highlighting as the editor**, including the current theme's colours:

```strings-halo3
Press &a_button to continue\r\nNext line\tTabbed text
Unnamed Unicode icon: &U+E453
Multiplayer message: #cause_player
```

- **Blue — glyph keywords**, such as `&a_button` and `&dpad_up`.
- **Green — Unicode references**, such as `&U+E453`. Numeric references preserve a specific character without choosing a glyph name.
- **Gold/orange — formatting escapes**, such as `\r`, `\n`, `\t`, `\\`, and `&&` (a literal ampersand).
- **Green — runtime placeholders**, such as `#cause_player` and `#score_to_win`, which the game fills in when displaying a message.
- **Red — unsupported keywords or invalid notation**. Hover the underline for an explanation. Text the import tool would refuse remains unsaved until fixed; empty or duplicate String IDs are also flagged.

Literal Unicode characters can be typed or pasted. Icon characters are converted to their appropriate notation; Japanese, Chinese and Korean text remains readable text.

### Completion and engine markup

- Type `&` to browse the game's glyph names, or `#` for multiplayer keywords. Use **↑/↓**, then **Enter** or **Tab** to insert. **Esc** closes suggestions. The popup's filter helps with long lists.
- Type `|` or `<` for engine markup. Examples include `|n` (line break), `|r` (align right), `|kAARRGGBB` (colour), `<color argb=#AARRGGBB>`, `<br/>`, and `<build_number/>`.
- Entities such as `&lt;`, Reach's `{cui:component.property}` bindings, and multiplayer placeholders are highlighted too. Reach's substitution pairs are rebuilt on edit, as its tool does on import.
- Markup the game may not interpret as intended gets an **amber underline** and a hover explanation, but can still be saved. Examples include an unsupported font, an incomplete colour escape, and a Halo 4 binding without a substitution pair.

Glyph tables are engine-specific: Halo 2, Halo 3, ODST and Reach have named glyphs; Reach's numbers differ from Halo 3's. Halo 4, Halo 2 Anniversary and Campaign Evolved currently accept `&&` and `&U+XXXX` because their named tables are unknown. In Halo CE text, `&` is an ordinary character.

## Selecting and editing rows

- With checkboxes hidden, click an **Index** or row background to select a row. Clicking a text field selects its row and begins editing.
- **Ctrl-click** toggles rows; **Shift-click** selects a range. Modified clicks in fields change selection without entering text editing.
- **Ctrl+A** selects all visible rows when the table has focus. Inside a field, it selects text instead.
- **View → Show Selection Checkboxes** makes row selection checkbox-driven. Editing or right-clicking another row preserves the checked selection. Check a row before dragging it.
- **View → Wrap Text** wraps both String ID and Text, expanding row heights without changing stored text. Both View preferences are remembered.
- Use **Add String** at the bottom for a new entry. Header and right-click actions insert, duplicate, or delete the selection. **Insert Above/Below** adds the same number of blank rows around each consecutive selected group.
- Drag a selected row's grabber to move the selection together across **every language**. With checkboxes hidden, dragging an unselected row selects and moves that row. Reordering is disabled while Find filters the list.
- **Delete** removes selected rows immediately. **Undo** restores them.

The footer shows the total and selected counts, including selected rows hidden by filtering. **Scroll to Top** is in the header; **Scroll to Bottom** is in the footer. Short lists place the footer immediately beneath the rows.

## Finding strings

**Find (Ctrl+F)** searches this tag's **String IDs** and/or **String Text**. Choose the current language, all languages, or a specific language. Next/previous results reveal the matching row and language without changing the row selection.

- **Filter Results** shows matching rows only. Searches use displayed glyph names and escapes; literal glyph queries also match their named or numeric notation. CE lists have no String IDs.
- **Within: Current Tag** updates results live.
- **Within: Selected Rows** runs when you press **Find**, or **Enter** in the search box. Each run captures the current selection—including hidden selected rows—and search options. Press **Find** again to refresh that scope.
- Changing the query or search options in Selected Rows clears the previous results and requires another **Find**. Changing only **Replace With** preserves the results.
- Editing a captured result keeps its row visible, even if it no longer matches, while clearing obsolete highlights. Insertions and reorders track the captured rows; deleted rows leave the results. A changed-results hint prompts a refresh. Find is disabled when no rows are selected.

Find from **Tag Fields** keeps the usual field-search options.

### Replacing matches

**Replace (Ctrl+H)** opens the Replace tab in the same floating window. The left-aligned Find and Replace tabs preserve the query, case/whole-word settings, and compatible scope and target options. **Ctrl+F** returns to Find.

Opening Replace with multiple rows selected in the Strings Editor defaults **Within** to **Selected Rows**. Compatible Find results carry over only for those selected rows; a new search still waits for Find.

Search instructions appear in button tooltips. All Tags search progress and unreadable-tag counts appear in the bottom status bar. Replacement failures and skipped locked matches appear in popups with an **OK** acknowledgment.

- **Replace** changes the current occurrence and advances past the inserted text. **Replace All** processes every original match in the selected scope once and reports the count.
- Every Replace scope uses **Find** (or **Enter** in the search box) to run or reload the search. Switching from Find carries over compatible results, the active match and Filter Results, so those matches can be replaced immediately. All Tags results cannot carry into Replace's Current Tag scope. **Filter Results** retains the captured rows/fields after replacement and while editing the pending search query or settings. In any scope with an explicit Find button, only the next Find applies those changes to filtering. Other edits may require refreshing before more replacements can be made.
- Routine tag-index refreshes preserve captured matches, navigation and filtering. Reloading or closing a searched document still invalidates its captured results.
- Replacement text is literal, with the Strings Editor's normal Unicode, glyph and escape conversion. An empty replacement removes the matched text; the search query must be nonempty.
- String IDs and text follow the selected rows and language options. ID changes do not update references elsewhere; duplicates retain their usual warning highlight.
- Every replacement is prepared before the operation changes anything. Invalid IDs or field values abort the entire operation. Invalid String Text notation (an unknown `&name` or malformed `&U+XXXX`) appears in the affected cells as red editable drafts; the stored text stays unchanged until those drafts are corrected and committed. Undo and Redo include these drafts. Finish or cancel pending field edits before replacing again.
- Changes remain unsaved. Each replacement action has one undo step. Undo and Redo preserve the captured Filter Results view while restoring replacement values. Undo also restores that pass's original matches and active position, allowing Replace or Replace All again without Find, provided the search and documents have not otherwise changed. Redo restores the post-replacement matches. Find reloads the matching rows and fields. Replace All across open tags has linked undo/redo, available from any affected tag when the action is next in every affected history. Closing or reloading a peer leaves local undo in the remaining tags.

In **Tag Fields**, Replace supports editable field values in **Current Tag** and **Open Tags**. Field names, block names and documentation remain Find-only. Returning to Find restores those target options.

Outside Expert mode, matches in normally locked fields are skipped while editable matches are replaced. A popup reports the skipped matches out of the total attempted. In Expert mode, replacing normally locked fields requires confirmation showing how many fields are affected. All replacement values are validated before that confirmation; cancelling or changing the search before confirming leaves the fields untouched.

**All Tags** supports Find only and requires pressing **Find** to run; typing or changing search options clears old results. Entering Replace from All Tags switches to Current Tag, which remains selected when returning to Find.

## Copy, cut and paste

Row shortcuts follow the active Strings Editor even when the pointer leaves its table. Inside a text field, normal text shortcuts take precedence. Text-field context menus include text clipboard actions alongside row actions.

- **Copy Rows (Ctrl+C)** captures the selected rows in order, including all languages. Copied rows have a **blue animated dashed outline**.
- **Cut Rows (Ctrl+X)** marks rows for a move with a **yellow animated dashed outline**. Both outlines use 6px dashes and gaps.
- **Esc** ends either mode and restores blank-row Insert actions without clearing the clipboard.
- **Paste Rows (Ctrl+V)** overwrites consecutive rows from the first selected row and appends past the end. **Insert Copied/Cut Rows Above** inserts before the first selected row; **Below** inserts after the last selected row.
- **Pasted rows become selected.** Duplicate IDs receive numbered suffixes, such as `message (1)`.
- A **successful cut move clears its clipboard and outline**. Another operation requires a new Copy or Cut. Overlapping moves use a snapshot and remove unused source rows.
- Editing a cut source cancels the move and keeps its snapshot as a normal copy. Cut also ends when its source becomes unavailable or the clipboard changes.

### Spreadsheet clipboard and cross-tag moves

Copy and Cut expose only the visible columns to external apps, **without column headers**: String ID and selected-language text, or just String Text for CE. Baboon's hidden clipboard payload retains all languages. Use **CSV Extract** to send all languages to a spreadsheet.

External spreadsheet pastes accept two columns for multilingual lists, or one for CE. Extra columns disable Paste. Blank IDs are generated, blank text clears a translation, and absent languages retain existing destination translations. Line breaks and literal Unicode are converted as needed.

Copy supports CE/multilingual conversion using the selected language. Multilingual columns match by language name. Unsupported source languages and destination languages absent from the source are reported. Unicode values are preserved; unsupported named keywords are red.

Cut moves require the same list type. Different language sets require a warning before the move. A cross-tag move has **linked undo/redo**, available from either tag when it is the next step in both histories. Closing one tag leaves independent undo in the remaining tag.

## Extracting and importing

**Extract** and **Import** are in the tag header and tag context menu. Choose the method, format, languages and destination. Choices survive ordinary tag reindexing.

### Baboon method

- **Text:** one UTF-16 file per language. Import also accepts UTF-8.
- **CSV:** UTF-8 with a BOM, one row per String ID and one text column per exported language. Commas, quotes and line breaks are escaped as CSV fields. Text uses the editor's glyph/escape notation; literal Unicode is accepted on import.
- Choose **Editing kit data folders** or a **Custom location**. Extract includes unsaved edits.
- Each import requires an explicit **Amend** or **Replace** choice and shows a preview of updates, additions, deletions, cleared translations, order changes and ignored columns. The result is one **undoable, unsaved edit**.

**Amend** preserves existing order and blank cells, updates matching IDs, and appends new IDs in file order.

**Replace** uses file order and deletes omitted IDs **from every language**. For retained IDs, absent or entirely blank language columns preserve existing translations. Individual blank cells clear translations only in Replace mode.

Halo CE CSV uses **Index** instead of String ID. Amend updates existing indices and appends rows with blank or absent Index values. Replace rebuilds the whole list in file order, with or without Index.

### Tool method

Tool mode is available for verified commands: **Halo 2 import/export** and **Halo CE import**. It uses saved files and native language folders. Directory imports may overwrite sibling tags and write directly to disk, without Baboon's import modes or undo.

**Save open edits first.** Clean open lists reload when Tool finishes; newer edits are preserved.

## Halo CE HUD messages

`hud_message_text` uses **Index**, **Name**, and **Text** columns. It supports the same row editing, clipboard, Find/Replace, and undo/redo actions as other string lists. Names occupy a 32-byte field: use 1–31 ASCII bytes without surrounding whitespace, control characters, or `=`. Renaming or deleting does not update external references.

Type `%` for button/action prompts such as `%action`, `%a-button`, and `%rotate-weapons`, or runtime substitutions `%custom-1` through `%custom-8`. Tokens are highlighted and completed; invalid tokens remain red drafts. `|n` is the engine line-break notation. `%%` in the editor or CSV preserves a literal percent sign in a text run. Ordinary CE Unicode lists keep treating `%` as literal text.

Native text transfers use UTF-16 `.hmt` files containing `name=text` lines, preserving whitespace and literal backslashes. CSV uses **Name,Text** headers and the editor's escape notation. Both provide the existing Amend/Replace preview and one undoable, unsaved import. Native HMT cannot represent actual embedded line breaks or literal percent escapes losslessly; use CSV for those values. Rare ambiguous token boundaries use braces, such as `%{back}-button` for the Back action followed by literal `-button`; this also requires CSV. Tool mode is unavailable for HUD messages.

The editor rebuilds the text blob and message elements together and validates the format limits before applying edits. Tags with malformed offsets, UTF-16, or unknown element codes cannot be edited through the Strings Editor; use Tag Fields to inspect them.


HUD message names do not make their indexes independent of order. Baboon checks HUD globals references to identify global interaction prompts, including references in open, unsaved HUD globals tags. These show a permanent fixed-index warning. Text and token edits remain available; adding, inserting, deleting, cutting, duplicating and reordering rows require **Expert Mode**. Imports and row pastes that preserve the same ordered message names can still update text in normal mode. Operations that change the slots are rejected before changing the document or undo history.

Mission HUD messages remain fully editable. Their HSC source names compile into numeric indexes: after inserting, deleting, reordering or renaming messages, recompile the referencing scenario's scripts before testing or rebuilding the map. Editing text in place does not require script recompilation. If Baboon cannot resolve how a tag is used, it displays a general index-safety warning instead of assuming the tag is a mission list. These warnings do not repair already-reordered tags or recompile scripts automatically.

## HUD message references

Weapon and equipment action-message fields, plus unit seat enter/boarding message fields, offer a **HUD Messages** picker in Halo 2, Halo 3, ODST, Reach, and Campaign Evolved. Halo 2 follows the saved `globals\globals` → `interface tags[0]/hud globals` → `new globals/hud text` reference chain. Later games follow `interface tags[0]/chud globals` and the skins’ `chud text` references. All skins must reference the same string list; differing or missing references show **Could not validate** until HUD-skin context is available. Rows show only the original String ID; hover for the English prompt. Search matches IDs and English text. IDs without English text remain selectable.

The adjacent **Open** button opens the resolved Unicode string list in the editor; Alt-click opens a floating window, like a Tag Reference row. The picker selects existing IDs; edit or add strings in the Strings Editor. Manual typing and pasting remain available. A missing ID is highlighted but does not block saving. **Could not validate** means a source reference or tag could not be loaded, rather than evidence that the ID is missing. Blank is the unset value.

These lookups use **saved tags only**. Opening HUD Messages reloads the saved reference chain and strings. Unsaved source-list or globals edits are deliberately excluded. HUD lookup polling and its periodic UI wake-ups are disabled. String ID type labels appear after the picker and open controls, keeping the buttons next to the value.


The HUD picker offers **Group by Prefix**. Prefixes shared by at least two IDs (before the first underscore) form submenus; singleton prefixes and IDs without a usable underscore prefix stay at the top level. This only changes the view: IDs stay intact. Search expands matching members inline beneath their headings, matching full IDs or English text, and the flat list remains available by turning grouping off. The grouping preference is remembered across HUD pickers for the current session. Popup height follows visible entries up to its scrolling limit and grows back when search is cleared. The adjacent Open action uses the same icon-only treatment as Tag Reference rows.

String ID browser buttons use the standard dropdown chevron. The HUD picker search input is 24px high and fills the popup width. Empty String ID fields display a 50%-opacity `NONE` placeholder, are treated as unset, and accept typing directly without deleting placeholder text.

All existing string lookup fields, including annotated ordinary text fields, share the same searchable popup presentation: a 24px search bar, content-sized search-popup width between 250px and 400px and height that follows visible entries, full-width left-aligned entries, and submenus sized to their longest visible label (capped at 400px), with half-opacity counts. Search examines selectable values at every hierarchy level and shows matching leaves beneath expanded headings. Selectable parent values remain available as entries within their own group. Generic lookup validation rules are unchanged. Picker captions describe their contents (for example, Materials and Object Functions), with matching search hints such as Search Materials and Search Functions. HUD Messages retains Search String IDs or Text; existing function and marker icons are reused, while Materials remains text-only. Truncated entries and group labels show their full text on hover; HUD entries combine a truncated ID with its existing English prompt in the same tooltip.
