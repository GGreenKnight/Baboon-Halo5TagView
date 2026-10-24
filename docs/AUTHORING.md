# Writing Baboon documentation

`help.json` defines the Docs sidebar sections. Existing `paragraph` and `bullets`
blocks remain plain text. To write a section in Markdown, reference a UTF-8 file
relative to the `docs` folder:

```json
{
  "title": "My section",
  "blocks": [
    { "kind": "markdown", "file": "my-section.md" }
  ]
}
```

Use headings, paragraphs, `**bold**`, `*italics*`, bullet or numbered lists,
inline backtick code, and fenced code blocks. Standard Markdown is parsed with
pulldown-cmark. The in-app renderer also supports strikethrough and horizontal
rules; HTML is shown literally. Links, images, and tables are not interactive
renderer features.

Use a fenced block with the language `strings-halo3` for a coloured string
example. It uses the Halo 3 string syntax and exactly the same highlighting and
theme colours as the Strings Editor. This is an application extension, since
standard Markdown does not define text colours. Ordinary code fences remain
plain monospace text.

See `strings-editor.md` for a complete example. Documentation files are loaded
once per app session; reopen Baboon after editing. A failed load is retried
when Help is opened again, allowing repaired missing files to recover. Build and release
packaging copy the entire `docs` folder automatically.
