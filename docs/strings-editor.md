Open a `multilingual_unicode_string_list`, or a Halo CE `unicode_string_list`, and choose **Strings Editor**. Each row shows its **Index**, **String ID** (multilingual lists), and the selected language's **Text**. *Index is the zero-based storage position.* **Tag Fields** shows the raw field tree.

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
- Editing a captured result keeps its row visible, even if it no longer matches, while clearing obsolete highlights. Insertions and reorders track the captured rows; deleted rows leave the results. A changed-results hint prompts a refresh. Find is disabled when no rows are selected.

Find from **Tag Fields** keeps the usual field-search options.

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
