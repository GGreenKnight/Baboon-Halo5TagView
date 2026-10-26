**Ctrl+F** opens Find for the active editor. **Ctrl+H** opens Replace. Both use the same window; switch tabs to keep the query and compatible options. The **About** icon beside Close opens this page.

## Finding matches

Enter the text to find and choose **Within** and **Look In**. **Match Case** and **Match Whole Word** refine the search. The counter and arrow buttons move between matches and reveal the active field or string row.

- **Tag Fields:** search the Current Tag, Open Tags, or All Tags. Find can search field names, field values, and blocks or documentation.
- **Strings Editor:** search the Current Tag or Selected Rows, targeting String IDs, String Text, or both. Language can be the current language, one specific language, or all languages. Halo CE string lists have no String IDs.
- Selecting multiple string rows before opening Replace defaults Within to **Selected Rows**.

Ordinary Current Tag and Open Tags Find searches update as you type. **All Tags**, **Selected Rows**, and every **Replace** scope require the **Find** button, or Enter in the Find field, to run or refresh the search.

**Filter Results** shows only the matching fields or string rows. In scopes with an explicit Find button, the filter stays tied to the last Find run while you edit the query or settings. Press Find to apply those changes. Replacement and its Undo/Redo keep the captured results visible, even after their values stop matching.

Search instructions are available in button tooltips. All Tags search progress and unreadable-tag counts appear in the bottom status bar.

## Replacing matches

Enter **Replace With**, then use **Replace** for the active occurrence or **Replace All** for the captured matches. Replacement text is literal. An empty replacement removes the matched text; the Find query must be nonempty.

Compatible results carry over when switching from Find to Replace. If multiple string rows are selected, carried results are narrowed to those rows. Otherwise, press Find before replacing. Newly inserted matches do not enter the same replacement pass.

- **Tag Fields:** only field values can be replaced, within the Current Tag or Open Tags. The fixed Field Values control has no dropdown. Field names, blocks, documentation, and All Tags remain Find-only.
- **Strings Editor:** replacements follow the selected rows, targets, and languages. String ID changes do not update references elsewhere.

Outside Expert mode, normally locked fields are skipped while editable matches are replaced. A popup reports the skipped matches out of the total attempted. In Expert mode, a confirmation lists how many normally locked fields will be affected before any replacement is applied.

Replacement failures and skipped-match notices appear in popups with an **OK** acknowledgment. Counts also appear in the bottom status bar.

## Glyphs, Unicode, and invalid text

String Text uses the Strings Editor's normal glyph and escape notation. Use a known keyword such as `&plasma_rifle`, a pasted Unicode character, or four-digit notation such as `&U+0123`. Keyword availability depends on the game. A literal ampersand is `&&`.

Unknown keywords and malformed Unicode notation appear as **red editable drafts** in the affected cells. The stored text stays unchanged until those drafts are corrected and committed. Undo and Redo include these drafts. Invalid String IDs or field values still abort the operation before anything changes. Finish or cancel pending edits before replacing again.

## Undo, Redo, and saving

Replacements remain **unsaved**. Each Replace or Replace All action has one undo step. Replace All across open tags has linked Undo/Redo, available from an affected tag when that action is next in every participating tag's history.

Undo restores the original matches, active position, and row highlight, allowing another replacement without Find when the search and documents have not otherwise changed. Redo restores the post-replacement matches. Filter Results remains active throughout.

Routine tag-index refreshes preserve captured results. Closing or reloading a searched document invalidates its capture; closing a peer from a linked action leaves local undo in the remaining tags.
