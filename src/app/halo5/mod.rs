//! Halo 5 source I/O coordination. Schema/data adaptation belongs to core;
//! the ordinary document editor owns all presentation and edit commands.
use super::*;
use crate::app::kits::KitStamp;
use crate::app::shell::spawn_worker;
use crate::core::source::{LoadedHalo5Tag, read_halo5_tag};
use blam_tags::halo5::Source;
#[cfg(test)]
mod tests;
impl Baboon {
    pub(in crate::app) fn begin_load_halo5_path(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.open_kit_for(&path) {
            return;
        }
        let kit = self.model.active_kit_id();
        let definitions = crate::core::bundled::locate_halo5_definitions();
        self.model.status = format!("Loading Halo 5: {}", path.display());
        spawn_worker(
            &self.tx,
            &ctx,
            move || WorkerMessage::SourceLoaded {
                kit,
                result: crate::core::source::load_halo5(path, &definitions)
                    .map_err(|e| format!("{e:#}")),
                recent_path: None,
            },
            move |error| WorkerMessage::SourceLoaded {
                kit,
                result: Err(error),
                recent_path: None,
            },
        );
    }
    pub(in crate::app) fn begin_halo5_tag_load(
        &mut self,
        key: String,
        index: usize,
        source: Arc<Source>,
        ctx: egui::Context,
    ) {
        let stamp = self.model.kit_stamp();
        let panic_key = key.clone();
        self.model.kits[self.model.active]
            .loading_tags
            .insert(key.clone());
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                let result = read_halo5_tag(&source, index).map_err(|e| format!("{e:#}"));
                WorkerMessage::Halo5TagLoaded { stamp, key, result }
            },
            move |error| WorkerMessage::Halo5TagLoaded {
                stamp,
                key: panic_key,
                result: Err(error),
            },
        );
    }
    pub(in crate::app) fn handle_halo5_tag_loaded(
        &mut self,
        stamp: KitStamp,
        key: String,
        result: Result<LoadedHalo5Tag, String>,
    ) -> bool {
        let Some(index) = self.model.resolve_stamp(stamp) else {
            return true;
        };
        let kit = &mut self.model.kits[index];
        kit.loading_tags.remove(&key);
        if !kit.open_tabs.contains(&key) {
            return true;
        }
        match result {
            Ok(doc) => {
                kit.parsed_tags.insert(key.clone(), doc.document);
                kit.halo5_documents.insert(key, doc.backing);
                self.model.status = "Halo 5 tag loaded".into();
            }
            Err(error) => self.model.status = format!("Could not load Halo 5 tag: {error}"),
        }
        false
    }
    pub(in crate::app) fn save_halo5_tag_by_key(
        &mut self,
        key: &str,
        save_as: bool,
    ) -> Result<PathBuf, String> {
        let kit = &mut self.model.kits[self.model.active];
        let entry = kit
            .entry_for_key(key)
            .cloned()
            .ok_or("Tag no longer exists")?;
        let TagEntryLocation::Halo5 { index } = entry.location else {
            return Err("Not a Halo 5 tag".into());
        };
        let Some(TagSource::Halo5 { source }) = kit.source.as_ref().map(|s| &s.source) else {
            return Err("Halo 5 source is not loaded".into());
        };
        let path = if !save_as {
            kit.halo5_save_paths.get(key).cloned()
        } else {
            None
        };
        let path = match path {
            Some(path) => path,
            None => {
                let name = entry
                    .display_path
                    .rsplit('/')
                    .next()
                    .unwrap_or("edited.tag");
                rfd::FileDialog::new()
                    .set_title("Save Halo 5 tag copy")
                    .set_file_name(name)
                    .save_file()
                    .ok_or("Save cancelled")?
            }
        };
        let original = &source.entries[index].path;
        if path.exists() && path.canonicalize().ok() == original.canonicalize().ok() {
            return Err("Choose a copy path; the loaded source is kept intact".into());
        }
        let doc = kit
            .halo5_documents
            .get_mut(key)
            .ok_or("Load the tag before saving")?;
        let document = kit
            .parsed_tags
            .get_mut(key)
            .ok_or("Load the tag before saving")?;
        doc.save(&document.tag, &path)
            .map_err(|e| format!("{e:#}"))?;
        document.mark_saved();
        kit.halo5_save_paths.insert(key.to_owned(), path.clone());
        Ok(path)
    }
}
