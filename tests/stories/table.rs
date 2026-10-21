use Area::{Cli, Edit, Editor, File, Game, Licence, Render, Scaffold, Sound};

pub const ALL_WRITTEN: bool = false;

pub const GPU: &str = "needs a GPU; run under pgpu";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Scaffold,
    File,
    Edit,
    Render,
    Editor,
    Licence,
    Cli,
    Sound,
    Game,
}

impl Area {
    pub const ALL: [Area; 9] = [
        Area::Scaffold,
        Area::File,
        Area::Edit,
        Area::Render,
        Area::Editor,
        Area::Licence,
        Area::Cli,
        Area::Sound,
        Area::Game,
    ];

    pub fn module(self) -> &'static str {
        match self {
            Area::Scaffold => "scaffold",
            Area::File => "file",
            Area::Edit => "edit",
            Area::Render => "render",
            Area::Editor => "editor",
            Area::Licence => "licence",
            Area::Cli => "cli",
            Area::Sound => "sound",
            Area::Game => "game",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Story {
    pub area: Area,
    pub name: &'static str,
    pub gpu: bool,
    pub hurts: &'static str,
}

const fn story(area: Area, name: &'static str, hurts: &'static str) -> Story {
    Story {
        area,
        name,
        gpu: false,
        hurts,
    }
}

const fn gpu(area: Area, name: &'static str, hurts: &'static str) -> Story {
    Story {
        area,
        name,
        gpu: true,
        hurts,
    }
}

pub const STORIES: &[Story] = &[
    story(
        Scaffold,
        "story_scaffold_new_game_builds_and_passes_its_own_tests",
        "a user's first pfx new gives a game that does not compile or fails its own tests",
    ),
    story(
        Scaffold,
        "story_scaffold_new_game_writes_the_files_its_readme_names",
        "the scaffold's README sends the user to a file or command the scaffold never wrote",
    ),
    story(
        Scaffold,
        "story_scaffold_new_game_first_scene_checks_clean",
        "the scaffold's first scene is refused by the same pfx that wrote it",
    ),
    story(
        Scaffold,
        "story_scaffold_new_game_with_module_writes_a_module_that_builds",
        "pfx new --with-module writes a gameplay module whose interface does not match the host",
    ),
    story(
        Scaffold,
        "story_scaffold_new_game_with_a_mark_uses_it",
        "pfx new --mark drops the user's mark or writes a broken image",
    ),
    story(
        Scaffold,
        "story_scaffold_new_game_at_a_path_writes_only_there",
        "pfx new --path writes outside the folder the user named",
    ),
    story(
        Scaffold,
        "story_scaffold_new_refuses_a_folder_that_holds_files",
        "pfx new writes over a user's existing work",
    ),
    story(
        Scaffold,
        "story_scaffold_new_refuses_a_name_that_is_not_a_crate_name",
        "pfx new writes a game whose Cargo.toml cargo refuses, with no word why",
    ),
    story(
        Scaffold,
        "story_scaffold_new_from_a_template_writes_a_document_that_checks_clean",
        "pfx new --from writes a .pitofx that pfx check refuses",
    ),
    story(
        Scaffold,
        "story_scaffold_new_from_a_template_with_a_seed_writes_the_same_bytes_twice",
        "the same template and seed give a different document on each run",
    ),
    story(
        Scaffold,
        "story_scaffold_new_from_an_unknown_template_is_refused",
        "a typo in --from writes a half document or the wrong one",
    ),
    story(
        Scaffold,
        "story_scaffold_new_from_a_template_json_names_the_file_written",
        "a script reading pfx new --from --json cannot find what was written",
    ),
    story(
        Scaffold,
        "story_scaffold_git_setup_makes_git_diff_show_a_pitofx_edit_as_toml",
        "a user reviewing a scene change in git sees binary noise instead of the changed TOML line",
    ),
    story(
        Scaffold,
        "story_scaffold_git_setup_keeps_the_users_own_gitattributes_lines",
        "pfx git setup drops lines the user wrote in .gitattributes",
    ),
    story(
        Scaffold,
        "story_scaffold_git_setup_run_twice_changes_nothing_the_second_time",
        "re-running pfx git setup duplicates lines in .gitattributes or the repository's config",
    ),
    story(
        Scaffold,
        "story_scaffold_git_setup_outside_a_repository_is_refused",
        "pfx git setup writes git config somewhere the user did not ask, or claims success with no repository",
    ),
    story(
        File,
        "story_file_print_shows_a_documents_recipe_as_toml",
        "a user cannot read what a .pitofx holds without the window",
    ),
    story(
        File,
        "story_file_print_all_lists_every_file_the_document_holds",
        "pfx scene print --all hides a file the document carries",
    ),
    story(
        File,
        "story_file_print_json_gives_a_table_as_json",
        "a script reading a document's table as JSON gets TOML or a partial table",
    ),
    story(
        File,
        "story_file_list_names_the_documents_entries",
        "pfx scene list misses an entry or lists one that is not there",
    ),
    story(
        File,
        "story_file_write_from_stdin_replaces_the_recipe_and_checks_it",
        "TOML piped into pfx scene write is kept without the check, so the next open fails",
    ),
    story(
        File,
        "story_file_bad_toml_on_stdin_leaves_the_document_byte_for_byte",
        "a bad write damages the user's only copy of a scene",
    ),
    story(
        File,
        "story_file_save_rebuilds_the_document_and_keeps_its_edits",
        "pfx scene save drops edits made in place",
    ),
    story(
        File,
        "story_file_save_with_a_thumbnail_stores_one",
        "pfx scene save --thumb writes no thumbnail or a broken one",
    ),
    story(
        File,
        "story_file_compact_shrinks_the_file_and_keeps_every_chunk",
        "compaction loses a chunk, or a file grows without bound under in-place edits",
    ),
    story(
        File,
        "story_file_many_in_place_edits_reopen_to_the_last_value",
        "a document edited many times opens to a stale value",
    ),
    story(
        File,
        "story_file_scene_hash_is_the_same_for_the_same_scene",
        "the same scene hashes differently, so caches and diffs treat it as changed",
    ),
    story(
        File,
        "story_file_fold_lists_applies_and_keeps_scene_edits",
        "pfx scene fold loses scene edits or applies them twice",
    ),
    story(
        File,
        "story_file_format_zero_document_migrates_to_the_current_format",
        "a user's oldest .pitofx no longer opens after an update",
    ),
    story(
        File,
        "story_file_migrated_document_reads_back_the_same_recipe",
        "migration silently changes a scene's values",
    ),
    story(
        File,
        "story_file_migrate_dry_run_writes_nothing",
        "pfx scene migrate --dry-run changes the user's files",
    ),
    story(
        File,
        "story_file_migrate_a_folder_migrates_every_document_in_it",
        "migrating a project folder skips a document or migrates one twice",
    ),
    story(
        File,
        "story_file_fix_repairs_a_fixable_scene_and_dry_run_writes_nothing",
        "pfx scene fix changes a scene it was only asked to report on, or leaves a fixable problem",
    ),
    story(
        File,
        "story_file_legacy_toml_imports_once_into_a_pitofx_beside_it",
        "an old recipe .toml is rewritten in place or imported again on every open",
    ),
    story(
        File,
        "story_file_from_a_newer_format_is_refused_naming_its_version",
        "an older pfx misreads a newer file instead of saying it needs an update",
    ),
    story(
        File,
        "story_file_truncated_is_refused_in_one_line",
        "a half-copied file crashes pfx or opens as an empty scene",
    ),
    story(
        File,
        "story_file_with_a_corrupt_chunk_is_refused_in_one_line",
        "a flipped byte is read as scene data instead of refused",
    ),
    story(
        File,
        "story_file_that_is_not_a_pitofx_is_refused_in_one_line",
        "a zip, a text file or an empty file passed as a scene panics or is overwritten",
    ),
    story(
        File,
        "story_file_missing_is_refused_naming_the_path",
        "a mistyped path gives a trace or creates a new empty document",
    ),
    story(
        Edit,
        "story_edit_set_then_get_round_trips_a_value_in_place",
        "an edit by dotted key is lost, lands on the wrong key or changes other values",
    ),
    story(
        Edit,
        "story_edit_set_several_keys_at_once_writes_all_or_none",
        "one bad key in a pfx set leaves the document half edited",
    ),
    story(
        Edit,
        "story_edit_set_an_unknown_key_is_refused_naming_it",
        "a misspelt key is written and silently ignored",
    ),
    story(
        Edit,
        "story_edit_set_a_value_of_the_wrong_type_is_refused",
        "a string where a number belongs is kept and breaks the next render",
    ),
    story(
        Edit,
        "story_edit_get_json_prints_the_value",
        "a script reading pfx get --json gets a value it cannot parse",
    ),
    story(
        Edit,
        "story_edit_add_then_remove_an_entry_leaves_the_document_as_it_was",
        "adding and removing an entry leaves debris in the recipe",
    ),
    story(
        Edit,
        "story_edit_remove_an_entry_others_refer_to_is_refused_naming_them",
        "removing a referenced entry leaves dangling names that break the scene",
    ),
    story(
        Edit,
        "story_edit_duplicate_copies_an_entry_with_the_keys_changed",
        "pfx duplicate drops keys or changes the original",
    ),
    story(
        Edit,
        "story_edit_duplicate_to_a_taken_name_is_refused",
        "pfx duplicate overwrites an existing entry",
    ),
    story(
        Edit,
        "story_edit_key_and_unkey_an_animated_property",
        "a keyed property animates wrong or a removed key stays",
    ),
    story(
        Edit,
        "story_edit_timeline_reports_frames_and_what_moves",
        "pfx timeline reports the wrong frame count or misses a keyed node",
    ),
    story(
        Edit,
        "story_edit_route_add_move_and_remove_points",
        "a route edit moves the wrong point or corrupts the route",
    ),
    story(
        Edit,
        "story_edit_route_point_out_of_range_is_refused",
        "an index past the route's end is written or panics",
    ),
    story(
        Edit,
        "story_edit_curve_add_move_and_remove_points",
        "a curve edit moves the wrong handle or corrupts the curve",
    ),
    story(
        Edit,
        "story_edit_align_distribute_and_ground_write_places",
        "align, distribute or ground puts objects in the wrong place",
    ),
    story(
        Edit,
        "story_edit_check_a_clean_document_and_a_document_with_problems",
        "pfx check passes a broken recipe or fails a clean one",
    ),
    story(
        Edit,
        "story_edit_build_writes_a_glb_and_its_sidecar",
        "pfx build writes no GLB, an invalid one, or none of its sidecar",
    ),
    story(
        Edit,
        "story_edit_build_twice_writes_the_same_bytes",
        "the same recipe builds a different GLB each time",
    ),
    story(
        Edit,
        "story_edit_diff_by_structure_finds_a_change_and_none_on_a_copy",
        "pfx diff misses a real change or reports one between identical files",
    ),
    story(
        Edit,
        "story_edit_import_a_glb_into_a_document",
        "importing a glTF binary loses meshes, materials or names",
    ),
    story(
        Edit,
        "story_edit_import_a_damaged_glb_is_refused",
        "a damaged GLB is imported as an empty or partial document",
    ),
    story(
        Edit,
        "story_edit_export_then_import_keeps_the_meshes",
        "a document exported to GLB and read back has lost geometry",
    ),
    story(
        Edit,
        "story_edit_export_only_one_object",
        "pfx export --only writes the whole scene or the wrong object",
    ),
    story(
        Edit,
        "story_edit_library_list_and_add_a_model",
        "a model added to the object library is missing from its list or index",
    ),
    story(
        Edit,
        "story_edit_credits_list_every_asset_and_refuse_an_unlicensed_one",
        "a scene ships an asset with no licence and no warning",
    ),
    gpu(
        Render,
        "story_render_live_still_of_a_sample_writes_its_png",
        "pfx shot --still on a shipped sample writes nothing or a blank frame",
    ),
    gpu(
        Render,
        "story_render_traced_still_of_a_sample_writes_its_png",
        "pfx render --recipe on a shipped sample fails or writes a blank frame",
    ),
    gpu(
        Render,
        "story_render_traced_still_writes_exr_and_hdr",
        "a still asked as EXR or HDR is written in another format or clipped",
    ),
    gpu(
        Render,
        "story_render_traced_still_with_alpha_writes_a_transparent_ground",
        "a transparent still comes out on an opaque ground",
    ),
    gpu(
        Render,
        "story_render_shot_writes_numbered_frames_a_manifest_and_audio",
        "pfx shot writes the wrong frame count, no manifest or no WAV",
    ),
    gpu(
        Render,
        "story_render_shot_writes_the_same_frames_twice",
        "the same scene and seed render different frames between runs",
    ),
    gpu(
        Render,
        "story_render_live_clip_writes_its_frames_and_manifest",
        "a live clip writes the wrong frame count or no manifest",
    ),
    gpu(
        Render,
        "story_render_traced_clip_writes_the_same_frames_twice",
        "the same scene, seed and revision render different frames, so a film flickers between runs",
    ),
    gpu(
        Render,
        "story_render_traced_clip_writes_a_video",
        "pfx clip --video writes no video or one with the wrong frame count",
    ),
    gpu(
        Render,
        "story_render_traced_clip_rerender_from_a_frame_keeps_earlier_frames",
        "a resumed clip rewrites or loses frames it had finished",
    ),
    gpu(
        Render,
        "story_render_traced_clip_passes_write_albedo_and_normal",
        "--passes writes no albedo or normal, or misaligned ones",
    ),
    gpu(
        Render,
        "story_render_clip_json_keeps_the_keys_callers_read",
        "a script or another tool reading pfx clip --json breaks on a renamed key",
    ),
    gpu(
        Render,
        "story_render_shoot_resumes_after_a_stop",
        "pfx shoot starts over instead of resuming, or skips frames",
    ),
    gpu(
        Render,
        "story_render_film_renders_a_script_to_its_out_folder",
        "pfx film writes no video or the wrong length",
    ),
    gpu(
        Render,
        "story_render_bench_writes_frame_stats",
        "pfx bench reports no stats or stats for the wrong frame count",
    ),
    gpu(
        Render,
        "story_render_bake_writes_its_bakes",
        "pfx bake writes nothing or bakes another scene",
    ),
    gpu(
        Render,
        "story_render_job_start_status_stop_and_resume",
        "a detached job cannot be followed, stopped or carried on from its last frame",
    ),
    gpu(
        Render,
        "story_render_stopped_clip_leaves_no_half_written_output",
        "a cancelled render leaves a truncated PNG or video that looks finished",
    ),
    gpu(
        Render,
        "story_render_past_the_memory_limit_is_refused_before_it_starts",
        "a render over --memory-limit runs out of VRAM mid-way instead of refusing up front",
    ),
    story(
        Render,
        "story_render_film_plan_prints_the_plan_without_rendering",
        "pfx film --plan renders or writes files",
    ),
    story(
        Render,
        "story_render_a_scene_that_does_not_parse_is_refused_with_its_cause",
        "a typo in a scene gives a crash or a generic error with no line to fix",
    ),
    story(
        Render,
        "story_render_an_unknown_camera_is_refused_naming_it",
        "a misspelt camera renders from a default view the user did not choose",
    ),
    story(
        Render,
        "story_render_a_bad_size_or_frame_count_is_refused",
        "a zero or malformed size or frame count starts a render that fails late",
    ),
    story(
        Render,
        "story_render_an_output_outside_tmp_is_refused",
        "a render writes into a folder the user did not mean, such as a project's sources",
    ),
    story(
        Render,
        "story_render_job_status_of_an_unknown_job_is_refused",
        "pfx job status on a wrong name prints a trace or another job",
    ),
    gpu(
        Editor,
        "story_editor_open_a_sample_draws_a_frame_offscreen",
        "the editor cannot open a shipped sample",
    ),
    gpu(
        Editor,
        "story_editor_open_the_start_scene_draws_a_frame",
        "the editor's start screen fails to draw",
    ),
    gpu(
        Editor,
        "story_editor_open_a_project_folder_draws_its_scene",
        "the editor cannot open a game's project folder",
    ),
    gpu(
        Editor,
        "story_editor_open_a_glb_draws_it_as_an_imported_document",
        "opening a GLB in the editor fails or loses its meshes",
    ),
    gpu(
        Editor,
        "story_editor_move_an_object_and_save_writes_the_pitofx",
        "a user's edit in the window is not what lands in the file",
    ),
    gpu(
        Editor,
        "story_editor_change_a_material_and_save_writes_it",
        "a material picked in the window is not the one saved",
    ),
    gpu(
        Editor,
        "story_editor_set_a_camera_from_the_view_and_save",
        "a camera set from the view saves another position",
    ),
    gpu(
        Editor,
        "story_editor_undo_then_redo_restores_the_edit",
        "undo or redo loses or doubles an edit",
    ),
    gpu(
        Editor,
        "story_editor_undo_back_to_the_start_matches_the_file_on_disk",
        "undoing every edit leaves a document that differs from the one opened",
    ),
    gpu(
        Editor,
        "story_editor_an_unsaved_edit_leaves_the_file_untouched",
        "the editor writes the file before the user saves",
    ),
    gpu(
        Editor,
        "story_editor_open_a_legacy_toml_imports_it_beside_it",
        "opening an old recipe in the editor rewrites the .toml",
    ),
    gpu(
        Editor,
        "story_editor_shading_clay_and_wireframe_draw_different_frames",
        "a view mode the user picks draws the same frame as another",
    ),
    gpu(
        Editor,
        "story_editor_bare_viewport_draws_no_panels",
        "--bare still draws panels over the picture",
    ),
    gpu(
        Editor,
        "story_editor_clip_of_a_tape_writes_its_frames",
        "a tape played through the editor writes the wrong frame count",
    ),
    story(
        Editor,
        "story_editor_a_corrupt_document_is_refused_without_drawing",
        "the editor opens a damaged file as an empty scene the user may then save over",
    ),
    story(
        Editor,
        "story_editor_a_bad_script_step_is_refused_naming_its_line",
        "a scripted edit with a typo runs part of the steps and saves",
    ),
    story(
        Editor,
        "story_editor_bad_shot_flags_are_refused",
        "a malformed --size, --fixture or --bare without a scene draws something anyway",
    ),
    story(
        Licence,
        "story_licence_free_without_a_key_shows_the_splash",
        "a build with no key hides the Powered by pfx splash",
    ),
    story(
        Licence,
        "story_licence_pro_customer_key_may_hide_the_splash",
        "a paying Pro customer cannot hide the splash they paid to hide",
    ),
    story(
        Licence,
        "story_licence_owner_key_may_hide_the_splash",
        "the owner's own products show the splash they are allowed to hide",
    ),
    story(
        Licence,
        "story_licence_pro_key_with_hide_off_shows_the_splash",
        "a Pro key hides the splash although the app did not ask to hide it",
    ),
    story(
        Licence,
        "story_licence_indie_key_keeps_the_splash",
        "an Indie key hides the splash, which only Pro may",
    ),
    story(
        Licence,
        "story_licence_friends_family_education_and_nonprofit_keys_keep_the_splash",
        "a free-of-charge Pro key hides the splash, which only a paying Pro customer or the owner may",
    ),
    story(
        Licence,
        "story_licence_unknown_group_keeps_the_splash",
        "a key with a group pfx does not know hides the splash",
    ),
    story(
        Licence,
        "story_licence_show_always_plays_the_splash_with_any_key",
        "an app that asks for the splash every time loses it because a key is set",
    ),
    story(
        Licence,
        "story_licence_timed_key_past_its_grace_shows_the_splash",
        "an expired yearly key keeps hiding the splash on new builds",
    ),
    story(
        Licence,
        "story_licence_timed_key_covers_builds_within_its_grace",
        "a user who paid loses the hidden splash on a build made inside the grace days",
    ),
    story(
        Licence,
        "story_licence_lifetime_key_covers_every_build",
        "a lifetime key stops hiding the splash on a later build",
    ),
    story(
        Licence,
        "story_licence_forged_or_tampered_key_is_refused",
        "an edited or self-signed key hides the splash",
    ),
    story(
        Licence,
        "story_licence_key_for_another_app_is_refused",
        "a key bought for another product unlocks pfx",
    ),
    story(
        Licence,
        "story_licence_key_of_another_version_or_shape_is_refused",
        "a key with a wrong version, prefix or shape is accepted",
    ),
    story(
        Licence,
        "story_licence_pasted_key_with_armour_and_line_breaks_is_read",
        "a key pasted from an email with its header lines and wraps is refused",
    ),
    story(
        Licence,
        "story_licence_unlock_with_a_bad_key_is_refused_in_one_line",
        "pfx settings unlock with a bad or unreadable key stores it or prints a trace",
    ),
    story(
        Licence,
        "story_licence_settings_show_the_licence_without_a_key",
        "pfx settings get licence --json reports a tier the user does not hold",
    ),
    story(
        Licence,
        "story_licence_badge_names_the_tier",
        "the licence badge shows a tier the user does not hold",
    ),
    story(
        Licence,
        "story_licence_splash_runs_its_whole_timeline",
        "the splash fades in, holds or fades out at the wrong times",
    ),
    story(
        Licence,
        "story_licence_chime_starts_on_the_frame_its_line_appears",
        "the splash chime drifts off the line it belongs to",
    ),
    story(
        Licence,
        "story_licence_chime_plays_once",
        "the chime plays twice or not at all in one splash",
    ),
    story(
        Licence,
        "story_licence_chime_level_ignores_every_app_setting",
        "an app's volume or mute setting silences or changes the splash chime",
    ),
    story(
        Licence,
        "story_licence_splash_cannot_be_skipped_before_it_ends",
        "a key press or click cuts the splash short",
    ),
    story(
        Licence,
        "story_licence_splash_waits_for_loading_before_it_ends",
        "the splash ends on a frame the game has not loaded",
    ),
    story(
        Licence,
        "story_licence_splash_background_colour_is_read_and_a_bad_one_refused",
        "a splash colour is ignored or a malformed one is kept",
    ),
    gpu(
        Licence,
        "story_licence_splash_draws_its_lockup",
        "the Powered by pfx lockup draws blank or cut off",
    ),
    story(
        Cli,
        "story_cli_unknown_subcommand_prints_clap_shape_and_exits_2",
        "a mistyped verb prints something scripts and users cannot recognise as a usage error",
    ),
    story(
        Cli,
        "story_cli_unknown_flag_prints_clap_shape_and_exits_2",
        "a mistyped flag is ignored or prints a trace instead of the usage error",
    ),
    story(
        Cli,
        "story_cli_missing_subcommand_prints_clap_shape_and_exits_2",
        "a verb with no subcommand dumps the whole help or does something",
    ),
    story(
        Cli,
        "story_cli_missing_value_prints_clap_shape_and_exits_2",
        "a flag with no value is read as the next word",
    ),
    story(
        Cli,
        "story_cli_unknown_subcommand_under_every_verb_prints_clap_shape",
        "one verb's subcommands answer a typo differently from the rest",
    ),
    story(
        Cli,
        "story_cli_usage_errors_are_plain_when_piped",
        "colour codes land in a log or a script's captured stderr",
    ),
    story(
        Cli,
        "story_cli_help_prints_the_whole_help_and_exits_0",
        "pfx --help fails or prints nothing a user can use",
    ),
    story(
        Cli,
        "story_cli_every_verb_prints_its_own_help",
        "pfx <verb> --help prints the wrong verb's flags",
    ),
    story(
        Cli,
        "story_cli_help_of_an_unknown_verb_is_a_usage_error",
        "pfx help <typo> prints the whole help as if it were fine",
    ),
    story(
        Cli,
        "story_cli_version_prints_the_release_version",
        "a user cannot tell which pfx they run when they report a problem",
    ),
    story(
        Cli,
        "story_cli_refusal_after_parsing_is_one_line_and_exits_1",
        "a refused command prints a backtrace or exits 2 like a usage error",
    ),
    story(
        Cli,
        "story_cli_settings_set_then_get_round_trips",
        "a setting the user sets is not the one the editor and games read",
    ),
    story(
        Cli,
        "story_cli_settings_set_a_bad_value_is_refused_and_keeps_the_old",
        "a bad value replaces a good setting",
    ),
    story(
        Cli,
        "story_cli_settings_set_an_unknown_key_is_refused",
        "a misspelt setting is stored and never read",
    ),
    story(
        Cli,
        "story_cli_settings_get_json_keeps_the_keys_callers_read",
        "a launcher or script reading pfx settings get --json breaks on a renamed key",
    ),
    story(
        Cli,
        "story_cli_cache_info_json_keeps_the_keys_callers_read",
        "a tool reading pfx cache info --json breaks on a renamed key",
    ),
    story(
        Cli,
        "story_cli_cache_clean_dry_run_removes_nothing",
        "pfx cache clean --dry-run deletes cache entries",
    ),
    story(
        Cli,
        "story_cli_cache_clean_a_bad_age_is_refused",
        "a malformed --older-than cleans everything",
    ),
    story(
        Cli,
        "story_cli_git_setup_json_keeps_the_keys_callers_read",
        "a script reading pfx git setup --json breaks on a renamed key",
    ),
    story(
        Cli,
        "story_cli_job_list_json_keeps_the_keys_callers_read",
        "a tool watching pfx job list --json breaks on a renamed key",
    ),
    story(
        Cli,
        "story_cli_credits_json_keeps_the_keys_callers_read",
        "a game's credits screen built from pfx credits --json loses an asset's licence",
    ),
    story(
        Cli,
        "story_cli_list_scenes_shots_and_presets",
        "pfx list prints nothing or entries that do not exist",
    ),
    story(
        Cli,
        "story_cli_font_subset_keeps_the_asked_characters",
        "a subset font drops a character the user asked to keep",
    ),
    story(
        Cli,
        "story_cli_font_subset_of_a_damaged_font_is_refused",
        "a damaged font is written out as an empty subset",
    ),
    story(
        Sound,
        "story_sound_mix_writes_the_same_wav_twice",
        "pfx sound mix writes a different WAV for the same script",
    ),
    story(
        Sound,
        "story_sound_mix_of_a_bad_script_is_refused",
        "a typo in a sound script writes silence",
    ),
    story(
        Sound,
        "story_sound_clean_writes_a_cleaned_wav_of_the_same_length",
        "pfx sound clean changes the length or channel count",
    ),
    story(
        Sound,
        "story_sound_clean_of_a_damaged_wav_is_refused",
        "a damaged WAV is cleaned into noise instead of refused",
    ),
    story(
        Cli,
        "story_cli_asset_list_and_sources_work_offline",
        "pfx asset list or sources needs the network to show what is on disk",
    ),
    story(
        Cli,
        "story_cli_steam_stage_of_a_bad_manifest_is_refused",
        "a broken steam.toml stages a depot anyway",
    ),
    story(
        Sound,
        "story_sound_compose_writes_a_cue_into_a_document",
        "pfx compose writes no score, or changes the scene beside it",
    ),
    story(
        Sound,
        "story_sound_compose_with_the_same_seed_writes_the_same_score",
        "the same style and seed compose different music",
    ),
    story(
        Sound,
        "story_sound_compose_list_and_print_show_styles_and_scores",
        "a user cannot see which styles and scores a document holds",
    ),
    story(
        Sound,
        "story_sound_compose_render_writes_the_same_wav_twice",
        "the same score renders different audio between runs",
    ),
    story(
        Sound,
        "story_sound_compose_analyze_derives_a_style_that_composes_again",
        "a derived style cannot be used to compose",
    ),
    story(
        Sound,
        "story_sound_compose_an_unknown_style_is_refused_naming_it",
        "a misspelt style composes from a default the user did not choose",
    ),
    story(
        Sound,
        "story_sound_compose_analyze_a_damaged_wav_is_refused",
        "a damaged stem is analysed into a nonsense style",
    ),
    story(
        Game,
        "story_game_headless_run_counts_the_frames_it_was_asked_for",
        "a game run headless for a time steps the wrong number of ticks",
    ),
    story(
        Game,
        "story_game_headless_run_twice_ends_in_the_same_state",
        "the same game, input and seed end in different states, so replays and tests drift",
    ),
    story(
        Game,
        "story_game_exit_code_reaches_the_caller",
        "a game's exit code is lost, so a launcher cannot tell a crash from a quit",
    ),
    story(
        Game,
        "story_game_input_recording_replays_the_same_actions",
        "a recorded session replays different actions",
    ),
    story(
        Game,
        "story_game_input_rebinding_changes_the_control_and_keeps_the_rest",
        "rebinding one action breaks another",
    ),
    story(
        Game,
        "story_game_input_glyphs_follow_the_device_family",
        "a controller shows another family's button glyphs",
    ),
    story(
        Game,
        "story_game_physics_steps_the_same_bodies_to_the_same_places",
        "the same physics world steps to different places between runs",
    ),
    story(
        Game,
        "story_game_physics_collision_reports_the_contact",
        "two bodies pass through each other with no contact event",
    ),
    story(
        Game,
        "story_game_sound_mixdown_of_the_same_timeline_is_the_same",
        "the same sounds mixed offline give different samples",
    ),
    story(
        Game,
        "story_game_sound_limiter_keeps_the_true_peak_under_the_ceiling",
        "a loud mix clips past the limiter's ceiling",
    ),
    story(
        Game,
        "story_game_scene_loads_from_a_pitofx",
        "a game cannot load a scene the editor saved",
    ),
    story(
        Game,
        "story_game_scene_with_problems_reports_each_with_its_place",
        "a broken scene loads silently or reports no place to fix",
    ),
    story(
        Game,
        "story_game_settings_round_trip_through_the_store",
        "a player's graphics settings are lost between runs",
    ),
    story(
        Game,
        "story_game_settings_frame_rate_and_vrr_are_the_only_pacing_choices",
        "a game offers a frame rate or sync choice pfx does not honour",
    ),
    story(
        Game,
        "story_game_module_runs_within_its_limits",
        "a gameplay module is refused although it stays within its limits",
    ),
    story(
        Game,
        "story_game_module_past_its_fuel_is_stopped_with_its_limit_named",
        "a runaway module hangs the game instead of being stopped",
    ),
    story(
        Game,
        "story_game_module_with_a_bad_manifest_is_refused",
        "a module with a broken manifest loads anyway",
    ),
    gpu(
        Game,
        "story_game_headless_with_a_gpu_reads_back_a_frame",
        "a game drawn offscreen reads back a blank frame",
    ),
    gpu(
        Game,
        "story_game_warm_compiles_before_the_first_frame",
        "the first frame stalls on pipeline compiles the warm-up should have done",
    ),
];
