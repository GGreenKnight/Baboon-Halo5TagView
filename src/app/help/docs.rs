//! Deserialization and state for Baboon's embedded help documentation.
//! It owns this focused support concern; application workflow coordination and unrelated UI behavior belong elsewhere.

use super::*;
use serde::Deserialize;

const HELP_DOCS_FILE: &str = "help.json";

#[derive(Clone)]
pub(in crate::app) enum HelpDocsState {
    Loaded(HelpDocs),
    Failed(String),
}

impl HelpDocsState {
    pub(in crate::app) fn load() -> Self {
        match load_help_docs() {
            Ok(docs) => HelpDocsState::Loaded(docs),
            Err(error) => HelpDocsState::Failed(error),
        }
    }
}

#[derive(Clone, Deserialize)]
pub(in crate::app) struct HelpDocs {
    pub(in crate::app) tabs: Vec<HelpDocTab>,
}

#[derive(Clone, Deserialize)]
pub(in crate::app) struct HelpDocTab {
    pub(in crate::app) id: String,
    pub(in crate::app) sections: Vec<HelpDocSection>,
}

#[derive(Clone, Deserialize)]
pub(in crate::app) struct HelpDocSection {
    pub(in crate::app) title: String,
    pub(in crate::app) blocks: Vec<HelpDocBlock>,
}

/// A hand-editable documentation block. The JSON uses an internal `"kind"`
/// tag, e.g. `{ "kind": "paragraph", "text": "..." }`, so future docs edits
/// do not need Rust-specific enum knowledge.
#[derive(Clone, Deserialize)]
#[serde(tag = "kind")]
pub(in crate::app) enum HelpDocBlock {
    /// Markdown may live in a separate, directly editable file beside help.json.
    #[serde(rename = "markdown")]
    Markdown { file: String, #[serde(skip)] text: String },
    #[serde(rename = "paragraph")]
    Paragraph { text: String },
    #[serde(rename = "bullets")]
    Bullets { items: Vec<String> },
}

impl HelpDocs {
    pub(in crate::app) fn tab(&self, id: &str) -> Option<&HelpDocTab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }
}

fn load_help_docs() -> Result<HelpDocs, String> {
    load_help_docs_from(&locate_help_docs_root())
}

fn load_help_docs_from(root: &Path) -> Result<HelpDocs, String> {
    let path = root.join(HELP_DOCS_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let mut docs = serde_json::from_str::<HelpDocs>(&text)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
    for block in docs.tabs.iter_mut().flat_map(|tab| &mut tab.sections).flat_map(|section| &mut section.blocks) {
        if let HelpDocBlock::Markdown { file, text } = block {
            let path = root.join(file);
            *text = std::fs::read_to_string(&path)
                .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        }
    }
    Ok(docs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_output_contains_and_loads_referenced_markdown() {
        // Tests normally find source docs through the development fallback.
        // Verify the build output explicitly, as the app runs beside that copy.
        let root = Path::new(env!("OUT_DIR")).join("docs");
        let docs = load_help_docs_from(&root).expect("copied help docs must load all referenced files");
        let section = docs.tab("doc").unwrap().sections.iter().find(|section| section.title == STRING_EDITOR_HELP_SECTION).unwrap();
        assert!(matches!(&section.blocks[0], HelpDocBlock::Markdown { text, .. } if text.contains("## Reading string text")));
    }
}
