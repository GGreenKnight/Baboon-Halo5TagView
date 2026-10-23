//! Finding a kit's folders and tools, refusing edits to a read-only kit, and
//! launching Sapien, Guerilla, tag_test, Blender and tool imports.

use super::*;

fn shortcut_startup_file(executable: &Path) -> Option<&'static str> {
    let name = executable.file_name()?.to_str()?;
    if name.eq_ignore_ascii_case("sapien.exe") {
        Some("editor_init.txt")
    } else if name.eq_ignore_ascii_case("tag_test.exe") || GameId::ALL.into_iter()
        .filter_map(GameFacts::tag_test_executable)
        .any(|executable| name.eq_ignore_ascii_case(executable)) {
        Some("init.txt")
    } else { None }
}

fn prepare_shortcut_startup(executable: &Path) -> Result<(), String> {
    if let Some(file) = shortcut_startup_file(executable) {
        let root = executable.parent().unwrap_or_else(|| Path::new("."));
        clear_scenario_startup_commands(&root.join(file))?;
    }
    Ok(())
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;
    use crate::app::shell::{AppAction, shortcuts::AppShortcut};

    #[test]
    fn shortcut_recognizes_all_supported_tool_executables() {
        for game in GameId::ALL {
            if let Some(name) = game.tag_test_executable() {
                assert_eq!(shortcut_startup_file(Path::new(name)), Some("init.txt"));
            }
        }
        assert_eq!(shortcut_startup_file(Path::new("TAG_TEST.EXE")), Some("init.txt"));
        assert_eq!(shortcut_startup_file(Path::new("SAPIEN.EXE")), Some("editor_init.txt"));
        assert_eq!(shortcut_startup_file(Path::new("blender.exe")), None);
        assert_eq!(shortcut_startup_file(Path::new("not_tag_test.exe")), None);
    }

    #[test]
    fn readded_shortcut_clears_its_own_init_and_preserves_other_settings() {
        for (exe, init, other) in [("halo3_tag_test.exe", "init.txt", "editor_init.txt"),
            ("sapien.exe", "editor_init.txt", "init.txt")] {
            let root = std::env::temp_dir().join(format!("baboon-shortcut-init-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let executable = root.join(exe);
            fs::write(&executable, []).unwrap();
            let original = b"; game_start commented\r\ndebug_objects 1\r\ngame_start levels\\old ; note\r\nmap_name old\r\n";
            fs::write(root.join(init), original).unwrap();
            fs::write(root.join(other), original).unwrap();
            let mut app = Baboon::for_test();
            let shortcut = AppShortcut { id: "readded".to_owned(), kit: None,
                name: "My renamed tool".to_owned(), path: executable };
            app.model.prefs.app_shortcuts.push(shortcut.clone());
            app.apply_app_action(AppAction::LaunchAppShortcut(0), &egui::Context::default());
            assert_eq!(fs::read(root.join(init)).unwrap(), b"; game_start commented\r\ndebug_objects 1\r\n; note\r\n");
            assert_eq!(fs::read(root.join(other)).unwrap(), original);
            // An edited default uses the same cleanup path as a re-added app.
            fs::write(root.join(init), original).unwrap();
            app.apply_app_action(AppAction::LaunchShortcutApp(shortcut), &egui::Context::default());
            assert_eq!(fs::read(root.join(init)).unwrap(), b"; game_start commented\r\ndebug_objects 1\r\n; note\r\n");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn shortcut_cleanup_leaves_unrelated_apps_and_missing_init_files_alone() {
        let root = std::env::temp_dir().join(format!("baboon-shortcut-init-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        prepare_shortcut_startup(&root.join("sapien.exe")).unwrap();
        assert!(!root.join("editor_init.txt").exists());
        let original = b"game_start levels\\keep\n";
        fs::write(root.join("init.txt"), original).unwrap();
        prepare_shortcut_startup(&root.join("blender.exe")).unwrap();
        assert_eq!(fs::read(root.join("init.txt")).unwrap(), original);
        fs::write(root.join("init.txt"), [0xff]).unwrap();
        assert!(prepare_shortcut_startup(&root.join("tag_test.exe")).is_err());
        assert_eq!(fs::read(root.join("init.txt")).unwrap(), [0xff]);
        fs::remove_dir_all(root).unwrap();
    }
}

impl Baboon {




    pub(in crate::app) fn refuse_read_only_edit(&mut self, kit_index: usize) -> bool {
        if self.model.editing_kit_is_read_only(kit_index) {
            self.model.status =
                "This editing kit is read-only. Change its Editing Kit settings to enable editing."
                    .to_owned();
            true
        } else {
            false
        }
    }







    pub(in crate::app) fn launch_sapien(&mut self) {
        self.launch_kit_tool("Sapien", "sapien.exe");
    }



    pub(in crate::app) fn launch_tag_test(&mut self) {
        self.launch_kit_tool("tag_test", self.model.tag_test_executable());
    }

    /// Re-added tools get the same startup cleanup as the built-in entries.
    pub(in crate::app) fn launch_shortcut_app(&mut self, label: &str, path: &Path) {
        if let Err(error) = crate::core::app_shortcut::validate_app_path(path) {
            self.model.status = error;
            return;
        }
        if let Err(error) = prepare_shortcut_startup(path) {
            self.model.status = error;
            return;
        }
        let root = std::path::absolute(path)
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        let options = if shortcut_startup_file(path).is_some()
            && root.as_ref().zip(self.model.editing_kit_root().as_ref())
                .is_some_and(|(selected, active)| same_recent_path(selected, active)) {
            self.model.active_kit_tool_folder_options()
        } else { Vec::new() };
        let mut command = match crate::core::app_shortcut::launch_command(path) {
            Ok(command) => command,
            Err(error) => { self.model.status = error; return; }
        };
        for (option, folder) in &options { command.arg(option).arg(folder); }
        if let Some(root) = root { command.current_dir(root); }
        match command.spawn() {
            Ok(_) => self.model.status = format!("Launched {label}"),
            Err(error) => {
                self.model.status = format!("Could not launch {label}: {error}");
                #[cfg(unix)]
                if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("exe")) {
                    self.model.status.push_str(". Windows applications require Wine.");
                }
            }
        }
    }

    pub(in crate::app) fn launch_kit_tool(&mut self, label: &str, executable_name: &str) {
        let Some(path) = self.model.kit_tool_path(executable_name) else {
            self.model.status = format!("{label} requires a loaded editing-kit folder");
            return;
        };
        if !path.is_file() {
            self.model.status = format!("{label} executable not found: {}", path.display());
            return;
        }
        self.launch_shortcut_app(label, &path);
    }

    /// Run a geometry Import request (`tool render/collision/physics/...`)
    /// streamed to the terminal panel.
    pub(in crate::app) fn process_pending_tool_import(&mut self, ctx: &egui::Context) {
        if self.model.editing_kit_is_read_only(self.model.active) {
            self.kit_tools.pending_tool_import = None;
            self.refuse_read_only_edit(self.model.active);
            return;
        }
        let Some(req) = self.kit_tools.pending_tool_import.take() else {
            return;
        };
        if self.model.editing_kit_root().is_none() {
            self.model.status = "Import requires a loaded editing-kit folder".to_owned();
            return;
        }
        let command = format!("tool {} \"{}\"", req.verb, req.source_dir);
        self.spawn_terminal_command(command, ctx.clone());
    }

    /// Queue the same editing-kit geometry import that a compatible tag
    /// reference offers, deriving the tool source folder from the clicked tag.
    pub(in crate::app) fn begin_reimport_geometry(&mut self, key: &str) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            self.model.status = "The tag is no longer in the browser".to_owned();
            return;
        };
        if !matches!(entry.location, TagEntryLocation::LooseFile(_)) {
            self.model.status = "Reimport requires a loose editing-kit tag".to_owned();
            return;
        }
        let Some(verb) = geometry_import_verb(self.model.names(), entry.group_tag) else {
            self.model.status = "This tag type does not support reimport".to_owned();
            return;
        };
        self.kit_tools.pending_tool_import = Some(ToolImportRequest {
            verb,
            source_dir: model_source_dir(&entry_rel_path(&entry)),
        });
    }

    /// Starts potentially expensive source or export work off the UI thread.
    /// The worker owns cloned inputs and reports status without mutating UI state.
    pub(in crate::app) fn begin_reimport_bitmap(&mut self, key: String, ctx: egui::Context) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        if self.kit_tools.terminal.running {
            self.model.status = "A command is already running".to_owned();
            return;
        }
        let Some(source) = self.model.source().map(|source| source.source.clone()) else {
            self.model.status = "Reimport requires a loaded editing-kit folder".to_owned();
            return;
        };
        let Some(entry) = self.model.entry_for_key(&key).cloned() else {
            self.model.status = "Bitmap tag is no longer in the source".to_owned();
            return;
        };
        let Some(tags_root) = (match &source {
            TagSource::LooseFolder { root, .. } => Some(root.as_path()),
            _ => None,
        }) else {
            self.model.status = "Bitmap reimport requires a loose tags folder".to_owned();
            return;
        };
        let Some(layout) = self.model.kit_layout_for(self.model.active) else {
            self.model.status = "Could not resolve editing-kit root".to_owned();
            return;
        };
        let work_dir = layout.root.clone();
        let Some(data_path) = bitmap_reimport_data_path(&entry, Some(tags_root)) else {
            self.model.status = "Could not resolve bitmap data path".to_owned();
            return;
        };
        let Some(game) = self.model.source().and_then(|source| source.game) else {
            self.model.status = "Bitmap reimport needs the kit's game".to_owned();
            return;
        };
        let plan = match bitmap_reimport_plan(game, &layout.data, &data_path) {
            Ok(plan) => plan,
            Err(error) => {
                self.model.status = error;
                return;
            }
        };
        let command = with_tool_folder_options(&plan.command, &self.model.active_kit_tool_folder_options());
        self.views[self.model.kits[self.model.active].id].terminal.open = true;
        self.kit_tools.terminal
            .lines
            .push(TerminalLineEntry::new(format!("> {command}")));
        trim_terminal_lines(&mut self.kit_tools.terminal.lines);
        self.kit_tools.terminal.scroll_to_bottom = true;
        self.kit_tools.terminal.refocus_input = true;
        self.kit_tools.terminal.running = true;
        self.model.status = format!("Reimporting bitmap {}", entry.display_path);
        let run_id = self.kit_tools.terminal.next_run_id;
        self.kit_tools.terminal.next_run_id = self.kit_tools.terminal.next_run_id.wrapping_add(1).max(1);
        let log_file = match create_terminal_log_file(run_id, &command) {
            Ok((path, file)) => {
                self.kit_tools.terminal.last_log_path = Some(path);
                Some(file)
            }
            Err(error) => {
                self.model.status = format!("Terminal full log unavailable: {error}");
                self.kit_tools.terminal.last_log_path = None;
                None
            }
        };

        let tx = self.tx.clone();
        let kit = self.model.active_kit_id();
        let panic_key = key.clone();
        let worker_ctx = ctx.clone();
        spawn_worker(
            &self.tx,
            &ctx,
            move || {
                // A source image missing from the data folder is written from
                // the tag's own color plate first (Halo CE, Halo 2).
                let recovered = match &plan.recover_to {
                    Some(path) => crate::app::export::write_bitmap_source(&source, &entry, path)
                        .map(|_| {
                            let _ = tx.send(WorkerMessage::TerminalLine(format!("Recovered the source image from the tag to {}", path.display())));
                            worker_ctx.request_repaint();
                        })
                        .map_err(|error| format!("Could not recover the source image from the tag: {error}")),
                    None => Ok(()),
                };
                let result = recovered
                    .and_then(|_| run_terminal_command_for_reimport(&command, &work_dir, &tx, &worker_ctx, log_file))
                    .and_then(|_| read_entry(&source, &entry).map_err(|error| error.to_string()));
                WorkerMessage::BitmapReimportFinished { kit, key, result }
            },
            move |error| WorkerMessage::BitmapReimportFinished {
                kit,
                key: panic_key,
                result: Err(format!("The reimport crashed: {error}")),
            },
        );
    }
}

impl Model {
    pub(in crate::app) fn editing_kit_root(&self) -> Option<PathBuf> {
        self.editing_kit_root_for(self.active)
    }

    pub(in crate::app) fn editing_kit_root_for(&self, kit_index: usize) -> Option<PathBuf> {
        Some(self.kit_layout_for(kit_index)?.root)
    }

    /// The loaded kit's root, tags and data folders. See [`KitLayout`].
    pub(in crate::app) fn kit_layout_for(&self, kit_index: usize) -> Option<KitLayout> {
        self.kits.get(kit_index)?.source.as_ref()?.kit_layout()
    }

    pub(in crate::app) fn kit_tool_path(&self, executable_name: &str) -> Option<PathBuf> {
        Some(self.editing_kit_root()?.join(executable_name))
    }

    /// The tag_test executable name for the loaded game. Each editing kit ships
    /// its own renamed build (e.g. H3EK is `halo3_tag_test.exe`); fall back to
    /// the generic name when the game is unknown.
    pub(in crate::app) fn tag_test_executable(&self) -> &'static str {
        self.tag_test_executable_for(self.active)
    }

    /// [`Self::tag_test_executable`] for the kit at `kit_index`.
    pub(in crate::app) fn tag_test_executable_for(&self, kit_index: usize) -> &'static str {
        tag_test_executable_for_game(
            self.kits.get(kit_index).and_then(|kit| kit.source.as_ref()).and_then(|s| s.game),
        )
    }

    pub(in crate::app) fn active_game_is_campaign_evolved(&self) -> bool {
        self.source_game().is_some_and(GameId::is_campaign_evolved)
    }

    pub(in crate::app) fn editing_kit_is_read_only(&self, kit_index: usize) -> bool {
        let Some(kit) = self.kits.get(kit_index) else {
            return false;
        };
        // The tags folder rather than the kit root: every folder above the
        // root is above the tags folder too, so this matches at least what the
        // root did and never makes a read-only kit writable.
        let root = self
            .kit_layout_for(kit_index)
            .map(|layout| layout.tags)
            .or_else(|| match &kit.source.as_ref()?.source {
                TagSource::SingleFile { path } => Some(path.clone()),
                _ => None,
            });
        self.prefs
            .custom_editing_kit_profiles
            .iter()
            .any(|profile| profile.is_read_only_for(kit.profile.as_ref(), root.as_deref()))
    }
}
