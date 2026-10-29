use super::*;
use crate::app::documents::{PendingCloseAction, SaveChangesPrompt};
use crate::app::loose_fixture::LooseKit;
use crate::core::document::apply::{DeferredOps, UndoStep};
use crate::core::document::ops::PendingFieldEdit;
use blam_tags::halo5::{Document, Field};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(crate::core::test_kits::unique_temp_dir("halo5-unified"))
    }
    fn tag(&self, name: &str, group: &str) -> PathBuf {
        let defs =
            blam_tags::halo5::Definitions::load(&crate::core::bundled::locate_halo5_definitions())
                .unwrap();
        let schema = defs.schema(group).unwrap();
        let mut bytes = vec![0; 128 + schema.size];
        bytes[..4].copy_from_slice(b"ucsh");
        for (offset, value) in [
            (4, 27u32),
            (32, 1),
            (36, 1),
            (60, 128),
            (64, schema.size as u32),
            (80, schema.size as u32),
            (120, u32::MAX),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[86] = 1;
        let path = self.0.join(format!("objects/{name}.{group}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn settle(app: &mut Baboon, ctx: &egui::Context) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app
        .model
        .kits
        .iter()
        .any(|k| (k.source.is_none() && k.requested_path.is_some()) || !k.loading_tags.is_empty())
    {
        if let Ok(message) = app.rx.recv_timeout(std::time::Duration::from_millis(25)) {
            app.apply_worker_message(message, ctx);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "load did not settle: {}",
            app.model.status
        );
    }
    app.process_worker_messages(ctx);
}
fn load(app: &mut Baboon, path: PathBuf, ctx: &egui::Context) -> String {
    app.begin_load_single_path(path, ctx.clone());
    settle(app, ctx);
    app.model.kits[app.model.active]
        .selected_key
        .clone()
        .inspect(|key| {
            assert!(
                app.model.active_kit().parsed_tags.contains_key(key),
                "{}",
                app.model.status
            )
        })
        .expect("single file opens a docked pane")
}
fn radius(app: &Baboon, key: &str) -> Field {
    app.model.kits[app.model.active].halo5_documents[key]
        .native
        .root_fields()
        .unwrap()
        .into_iter()
        .find(|f| f.name.starts_with("bounding radius"))
        .unwrap()
}
#[test]
fn halo5_opens_in_the_normal_workspace_and_docked_tag_tree() {
    let fixture = Fixture::new();
    let path = fixture.tag("grunt_mecha", "biped");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    let key = load(&mut app, path.clone(), &ctx);
    let kit = &app.model.kits[0];
    assert!(matches!(
        kit.source.as_ref().unwrap().source,
        TagSource::Halo5 { .. }
    ));
    assert!(kit.halo5_documents.contains_key(&key));
    assert!(
        kit.parsed_tags.contains_key(&key),
        "same document model as the other games"
    );
    assert_eq!(kit.open_tabs, vec![key.clone()]);
    assert!(app.views[kit.id].tabs_from_tree().contains(&key));
    assert_eq!(kit.requested_path, Some(path));
    assert!(!kit.is_empty_workspace());
    assert_eq!(
        kit.source.as_ref().unwrap().entries[0].group_tag,
        u32::from_be_bytes(*b"bipd")
    );
}
#[test]
fn halo3_and_halo5_share_workspace_layout_and_keep_separate_documents() {
    let h3 = LooseKit::new("halo3-unified", "halo3_mcc");
    let h3path = h3.write_mcc("objects/brute", "biped", |_| {});
    let h5 = Fixture::new();
    let h5path = h5.tag("grunt_mecha", "biped");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    let h3key = load(&mut app, h3path, &ctx);
    let h3id = app.model.active_kit_id();
    let h5key = load(&mut app, h5path.clone(), &ctx);
    let h5id = app.model.active_kit_id();
    assert_ne!(h3id, h5id);
    assert_eq!(app.model.kits.len(), 2);
    assert!(app.model.kits[0].parsed_tags.contains_key(&h3key));
    assert!(app.model.kits[1].parsed_tags.contains_key(&h5key));
    assert!(app.views[h3id].tabs_from_tree().contains(&h3key));
    assert!(app.views[h5id].tabs_from_tree().contains(&h5key));
    app.begin_load_single_path(h5path, ctx.clone());
    assert_eq!(app.model.kits.len(), 2);
    assert_eq!(app.model.active_kit_id(), h5id);
}
#[test]
fn halo5_folder_uses_shared_browser_and_supports_multiple_tag_tabs() {
    let fixture = Fixture::new();
    fixture.tag("grunt_mecha", "biped");
    fixture.tag("beam_arm", "weapon");
    // Even an unreadable archive beside the tags must never be mounted.
    fs::write(fixture.0.join("archive.module"), b"mohd\x17\0\0\0").unwrap();
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    app.begin_load_folder_path(fixture.0.clone(), ctx.clone());
    settle(&mut app, &ctx);
    let keys: Vec<_> = app
        .model
        .source()
        .unwrap()
        .entries
        .iter()
        .map(|e| e.key.clone())
        .collect();
    assert_eq!(keys.len(), 2);
    let TagSource::Halo5 { source } = &app.model.source().unwrap().source else {
        panic!("expected Halo 5 tags");
    };
    assert!(
        source.skipped.is_empty(),
        "archives are ignored, not parsed"
    );
    for key in &keys {
        app.select_entry(key.clone(), ctx.clone());
        settle(&mut app, &ctx);
    }
    let kit = app.model.active_kit();
    assert_eq!(kit.halo5_documents.len(), 2);
    assert_eq!(kit.open_tabs.len(), 2);
    assert_eq!(app.views[kit.id].tabs_from_tree().len(), 2);
    assert!(!app.model.source().unwrap().group_tree.children.is_empty());
}
#[test]
fn halo5_application_loader_accepts_tags_and_refuses_archives() {
    let fixture = Fixture::new();
    let path = fixture.0.join("archive.module");
    fs::write(&path, b"mohd\x17\0\0\0").unwrap();
    assert!(!blam_tags::halo5::is_halo5_file(&path));
    assert!(!blam_tags::halo5::is_halo5_folder(&fixture.0));
    let error =
        crate::core::source::load_halo5(path, &crate::core::bundled::locate_halo5_definitions())
            .err()
            .expect("archives are outside the tag-only application loader");
    assert!(error.to_string().contains("extracted Halo 5 tag file"));
}
#[test]
fn edits_use_shared_undo_save_dirty_indicators_and_close_prompt() {
    let fixture = Fixture::new();
    let path = fixture.tag("grunt_mecha", "biped");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    let key = load(&mut app, path.clone(), &ctx);
    let field = radius(&app, &key);
    let original = fs::read(&path).unwrap();
    let path_in_tag = app.model.active_kit().parsed_tags[&key]
        .tag
        .root()
        .fields()
        .find(|field| field.name().starts_with("bounding radius"))
        .map(|field| crate::core::document::value::append_field_path_for("", &field))
        .unwrap();
    app.apply_doc_ops(
        app.model.active,
        &key,
        "Edit",
        DeferredOps {
            pending: vec![PendingFieldEdit {
                path: path_in_tag,
                input: "1.5".into(),
            }],
            ..Default::default()
        },
        UndoStep::Own,
    );
    assert!(app.model.tag_is_dirty(&key));
    assert!(app.model.any_kit_dirty());
    assert!(app.can_undo_current());
    app.undo_current_tag();
    assert!(!app.model.tag_is_dirty(&key));
    app.redo_current_tag();
    assert!(app.model.tag_is_dirty(&key));
    app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx);
    let prompt = app
        .dialogs
        .get::<SaveChangesPrompt>()
        .expect("same save prompt as MCC");
    assert_eq!(prompt.dirty_tags.len(), 1);
    assert_eq!(prompt.dirty_tags[0].tag_id, key);
    app.dialogs.close::<SaveChangesPrompt>();
    let output = fixture.0.join("copy.biped");
    app.model.kits[0]
        .halo5_save_paths
        .insert(key.clone(), output.clone());
    assert_eq!(app.save_tag_by_key(&key).unwrap(), output);
    assert!(!app.model.tag_is_dirty(&key));
    assert_eq!(fs::read(&path).unwrap(), original);
    let schema = app.model.kits[0].halo5_documents[&key]
        .native
        .schema
        .clone();
    let reopened = Document::open(&output, schema).unwrap();
    assert_eq!(
        &reopened.bytes()[field.offset..field.offset + field.size],
        &1.5f32.to_le_bytes()
    );
    app.request_close_action(PendingCloseAction::CloseTab(key.clone()), &ctx);
    assert!(!app.model.kits[0].halo5_documents.contains_key(&key));
    assert!(
        !app.views[app.model.kits[0].id]
            .tabs_from_tree()
            .contains(&key)
    );
}
fn texts(shapes: &[egui::epaint::ClippedShape]) -> String {
    fn collect(shape: &egui::epaint::Shape, out: &mut String) {
        match shape {
            egui::epaint::Shape::Text(text) => {
                out.push_str(text.galley.job.text.as_str());
                out.push('\n');
            }
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut result = String::new();
    for shape in shapes {
        collect(&shape.shape, &mut result);
    }
    result
}
#[test]
fn unified_tag_pane_renders_flags_enums_and_components_with_shared_controls() {
    let fixture = Fixture::new();
    let path = fixture.tag("grunt_mecha", "biped");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    ctx.set_fonts(foundation_fonts());
    load(&mut app, path, &ctx);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(2560.0, 1380.0),
        )),
        ..Default::default()
    };
    let output = crate::app::run_ui_test(&ctx, input, |ui| app.run_frame(ui));
    let shown = texts(&output.shapes);
    assert!(shown.contains("bounding radius"), "{shown}");
    assert!(shown.contains("lightmap shadow mode"), "{shown}");
    assert!(
        shown.contains("always render in floating shadow"),
        "flags appear as named checkboxes: {shown}"
    );
    assert!(
        shown.contains("grunt_mecha.biped"),
        "normal browser and tab title: {shown}"
    );
}
#[test]
fn documents_without_preview_data_use_the_shared_field_tree_for_bitmaps() {
    let fixture = Fixture::new();
    let path = fixture.tag("example", "bitmap");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    ctx.set_fonts(foundation_fonts());
    load(&mut app, path, &ctx);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(2560.0, 1380.0),
        )),
        ..Default::default()
    };
    let output = crate::app::run_ui_test(&ctx, input, |ui| app.run_frame(ui));
    let shown = texts(&output.shapes);
    assert!(shown.contains("Usage"), "{shown}");
    assert!(shown.contains("Diffuse Map"), "{shown}");
}
#[test]
fn reported_real_biped_uses_the_unified_viewer() {
    let Some(root) = std::env::var_os("HALO5_TEST_DUMP") else {
        return;
    };
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    let path =
        PathBuf::from(root).join("tag_dump/objects/characters/grunt_mecha/grunt_mecha.biped");
    let key = load(&mut app, path, &ctx);
    let field = radius(&app, &key);
    assert_eq!(
        &app.model.active_kit().halo5_documents[&key].native.bytes()
            [field.offset..field.offset + field.size],
        &1.1483762f32.to_le_bytes()
    );
    assert!(
        app.views[app.model.active_kit_id()]
            .tabs_from_tree()
            .contains(&key)
    );
}
#[test]
fn real_references_use_shared_rows_and_open_the_referenced_tag_in_the_workspace() {
    let Some(root) = std::env::var_os("HALO5_TEST_DUMP") else {
        return;
    };
    let path =
        PathBuf::from(root).join("tag_dump/objects/characters/grunt_mecha/grunt_mecha.biped");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    ctx.set_fonts(foundation_fonts());
    let key = load(&mut app, path, &ctx);
    let kit_id = app.model.active_kit_id();
    let tag = &app.model.active_kit().parsed_tags[&key].tag;
    let model = tag.root().field_path("model").unwrap();
    assert_eq!(model.field_type(), blam_tags::TagFieldType::TagReference);
    let Some(blam_tags::TagFieldData::TagReference(reference)) = model.value() else {
        panic!("reference value missing")
    };
    let (group, name) = reference.group_tag_and_name.unwrap();
    assert_eq!(group, u32::from_be_bytes(*b"hlmt"));
    assert_eq!(name, r"objects\characters\grunt_mecha\grunt_mecha");
    let Some(blam_tags::TagFieldData::TagReference(null)) =
        tag.root().field_path("crate object").unwrap().value()
    else {
        panic!("null reference missing")
    };
    assert!(null.group_tag_and_name.is_none());
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(2560.0, 8000.0),
        )),
        ..Default::default()
    };
    let output = crate::app::run_ui_test(&ctx, input, |ui| app.run_frame(ui));
    let shown = texts(&output.shapes);
    assert!(
        shown.contains(r"objects\characters\grunt_mecha\grunt_mecha.model"),
        "{shown}"
    );
    assert!(shown.contains("tag reference"), "{shown}");
    assert!(shown.contains("(no reference)"), "{shown}");
    app.references.pending_open = Some(crate::app::editor::OpenTagRequest {
        group_tag: group,
        rel_path: name,
        float: false,
    });
    app.process_pending_open(&ctx);
    settle(&mut app, &ctx);
    assert_eq!(app.model.active_kit_id(), kit_id);
    assert_eq!(app.model.kits.len(), 1);
    let target_key = app.model.active_kit().selected_key.clone().unwrap();
    assert_ne!(target_key, key);
    assert!(
        app.model.active_kit().parsed_tags.contains_key(&target_key),
        "{}",
        app.model.status
    );
    assert!(app.model.active_kit().parsed_tags.contains_key(&key));
    assert!(
        app.model
            .entry_for_key(&target_key)
            .unwrap()
            .display_path
            .ends_with("grunt_mecha.model")
    );
    // Opening the same reference again reuses the document and tab.
    let count = app.model.active_kit().open_tabs.len();
    app.references.pending_open = Some(crate::app::editor::OpenTagRequest {
        group_tag: group,
        rel_path: r"objects\characters\grunt_mecha\grunt_mecha".into(),
        float: false,
    });
    app.process_pending_open(&ctx);
    assert_eq!(app.model.active_kit().open_tabs.len(), count);
}
#[test]
fn stale_halo5_load_results_cannot_modify_a_replaced_workspace() {
    let fixture = Fixture::new();
    let path = fixture.tag("grunt_mecha", "biped");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    let key = load(&mut app, path, &ctx);
    let stale = app.model.kit_stamp();
    app.model.kits[0].generation += 1;
    assert!(app.handle_halo5_tag_loaded(stale, key, Err("stale".into())));
}
#[test]
fn shared_discard_reloads_original_data_clears_view_state_and_retains_copy_destination() {
    let fixture = Fixture::new();
    let path = fixture.tag("grunt_mecha", "biped");
    let original = fs::read(&path).unwrap();
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    let key = load(&mut app, path.clone(), &ctx);
    let field_path = app.model.active_kit().parsed_tags[&key]
        .tag
        .root()
        .fields()
        .find(|field| field.name().starts_with("bounding radius"))
        .map(|field| crate::core::document::value::append_field_path_for("", &field))
        .unwrap();
    app.apply_doc_ops(
        0,
        &key,
        "Edit",
        DeferredOps {
            pending: vec![PendingFieldEdit {
                path: field_path.clone(),
                input: "1.5".into(),
            }],
            ..Default::default()
        },
        UndoStep::Own,
    );
    assert!(app.model.tag_is_dirty(&key));
    let copy = fixture.0.join("copy.biped");
    app.model.kits[0]
        .halo5_save_paths
        .insert(key.clone(), copy.clone());
    let pane_key = format!("pane\u{1f}{key}");
    let draft_key = format!("{key}|{field_path}");
    let view = &mut app.views[app.model.active_kit_id()];
    view.row_heights
        .insert(pane_key.clone(), Default::default());
    view.find_filter_applied.insert(
        key.clone(),
        crate::app::editor::AppliedFindFilter {
            signature: "bounding radius".into(),
            filter: Arc::new(Default::default()),
        },
    );
    view.edit_buffers.draft_mut(&draft_key, "1.5").text = "2.5".into();
    app.discard_tag_changes(0, &key, &ctx);
    let view = &mut app.views[app.model.active_kit_id()];
    assert!(!view.row_heights.contains_key(&pane_key));
    assert!(!view.find_filter_applied.contains_key(&key));
    assert_eq!(view.edit_buffers.draft_mut(&draft_key, "0").text, "0");
    settle(&mut app, &ctx);
    assert!(!app.model.tag_is_dirty(&key));
    assert!(!app.can_undo_current());
    assert_eq!(
        app.model.active_kit().halo5_documents[&key].native.bytes(),
        original
    );
    assert_eq!(app.model.active_kit().halo5_save_paths[&key], copy);
    assert!(app.model.active_kit().open_tabs.contains(&key));
    assert_eq!(fs::read(&path).unwrap(), original);
}
#[test]
fn shared_drafts_and_find_work_on_the_adapted_document() {
    use crate::app::editor::{DraftCommit, DraftFlush, compute_find_field_filter};
    let fixture = Fixture::new();
    let path = fixture.tag("grunt_mecha", "biped");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    let key = load(&mut app, path, &ctx);
    let tag = &app.model.active_kit().parsed_tags[&key].tag;
    let path = tag
        .root()
        .fields()
        .find(|f| f.name().starts_with("bounding radius"))
        .map(|field| crate::core::document::value::append_field_path_for("", &field))
        .unwrap();
    let filter = compute_find_field_filter(
        tag,
        &app.model.active_kit().names,
        None,
        "bounding radius",
        crate::app::search::FindLookIn::default(),
        false,
        false,
    );
    assert!(
        filter
            .visible_paths
            .iter()
            .any(|p| p.contains("bounding radius"))
    );
    let draft_key = format!("{key}|{path}");
    let draft = app.views[app.model.active_kit_id()]
        .edit_buffers
        .draft_mut(&draft_key, "0");
    draft.text = "2.5".into();
    draft.changed = true;
    draft.keep_commit(|| {
        DraftCommit::new(&key, vec![draft_key.clone()], move |texts| {
            Ok(DeferredOps {
                pending: vec![PendingFieldEdit {
                    path: path.clone(),
                    input: texts[0].into(),
                }],
                ..Default::default()
            })
        })
    });
    assert!(app.commit_drafts(DraftFlush::All));
    assert!(app.model.tag_is_dirty(&key));
    assert!(app.can_undo_current());
    app.undo_current_tag();
    assert!(!app.model.tag_is_dirty(&key));
}
#[test]
fn shared_edit_commands_respect_the_backends_field_and_block_capabilities() {
    use crate::core::document::ops::{BlockOp, BlockOpKind};
    let fixture = Fixture::new();
    let path = fixture.tag("grunt_mecha", "biped");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    let key = load(&mut app, path, &ctx);
    let doc = &app.model.active_kit().parsed_tags[&key];
    let read_only = doc
        .tag
        .root()
        .fields()
        .find(|f| f.name().starts_with("runtime object type"))
        .map(|field| crate::core::document::value::append_field_path_for("", &field))
        .unwrap();
    let block = doc
        .tag
        .root()
        .fields()
        .find(|f| f.as_block().is_some())
        .map(|field| crate::core::document::value::append_field_path_for("", &field))
        .unwrap();
    app.model.prefs.expert_mode = true;
    let applied = app
        .apply_doc_ops(
            app.model.active,
            &key,
            "Edit",
            DeferredOps {
                pending: vec![PendingFieldEdit {
                    path: read_only,
                    input: "1".into(),
                }],
                ..Default::default()
            },
            UndoStep::Own,
        )
        .unwrap();
    assert!(applied.outcomes[0].result.is_err());
    app.apply_doc_ops(
        app.model.active,
        &key,
        "Add element",
        DeferredOps {
            block_ops: vec![BlockOp {
                path: block,
                kind: BlockOpKind::Add,
            }],
            ..Default::default()
        },
        UndoStep::Own,
    );
    assert!(!app.model.tag_is_dirty(&key));
    assert!(!app.can_undo_current());
}
#[test]
fn real_function_blocks_render_through_the_existing_curve_viewer() {
    let Some(root) = std::env::var_os("HALO5_TEST_DUMP") else {
        return;
    };
    let path =
        PathBuf::from(root).join("tag_dump/objects/characters/grunt_mecha/grunt_mecha.biped");
    let mut app = Baboon::for_test();
    let ctx = egui::Context::default();
    ctx.set_fonts(foundation_fonts());
    let key = load(&mut app, path, &ctx);
    let tag = &app.model.active_kit().parsed_tags[&key].tag;
    let field = tag
        .root()
        .field_path("functions[0]/default function/data")
        .unwrap();
    assert!(field.is_function_data());
    assert_eq!(field.as_data().unwrap().len(), 52);
    assert!(field.as_function().unwrap().evaluate(0.5, 0.5).is_finite());
    app.views[app.model.active_kit_id()]
        .pending_expand
        .insert(key.clone(), true);
    crate::app::editor::FUNCTION_PREVIEWS_BUILT.with(|count| count.set(0));
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(2560.0, 8000.0),
        )),
        ..Default::default()
    };
    let output = crate::app::run_ui_test(&ctx, input, |ui| app.run_frame(ui));
    assert!(
        crate::app::editor::FUNCTION_PREVIEWS_BUILT.with(|count| count.get()) >= 14,
        "the standard function renderer builds the biped's function graphs"
    );
    let shown = texts(&output.shapes);
    assert!(shown.contains("default function"), "{shown}");
    assert!(
        !app.model.tag_is_dirty(&key),
        "viewing the curves does not edit their payloads"
    );
}
