# Slice 5 coverage — MCP tools, size limits, error kinds, version conflicts

Status: complete for the shipped v1 agent surface
Updated: 2026-08-19

Every row below names where the behavior is documented and the test that holds
it in place. "Docs" paths are section headings; "Test" paths are the test
function name in that file. Rust unit tests live in the `mod tests` block at the
bottom of the named source file.

The tool table itself is pinned: `mcp.rs::the_advertised_tool_surface_is_exactly_the_documented_one`
compares `tools/list` against the 37 names below and against the published
command list in `schemas/v1/local-api.json`, so a tool cannot be added or
renamed without this file being revisited.

## MCP tools (37)

All tests in this table live in `src-tauri/tests/mcp.rs` unless another file is
named. Docs references are to `docs/LOCAL_API.md` unless another file is named.

### Read tools

| Tool | Documented at | Tested at |
| --- | --- | --- |
| `search_records` | "Initial tools → Read" | `search_never_returns_more_than_fifty_rows` |
| `list_contacts` | "Initial tools → Read" | `list_tools_take_a_limit_and_refuse_an_unusable_one` |
| `get_contact` | "Initial tools → Read" | `a_missing_record_maps_to_not_found`, `a_draft_can_be_proposed_applied_and_undone_over_mcp` |
| `list_companies` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface` |
| `get_company` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface` |
| `list_opportunities` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface` |
| `get_opportunity` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface` |
| `get_timeline` | "Initial tools → Read", "Agent onboarding" (bounds) | `a_timeline_is_capped_and_its_bodies_truncated` |
| `list_tasks` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface`, `list_tools_take_a_limit_and_refuse_an_unusable_one` |
| `get_attention_flags` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface`, `explain_attention_flag_answers_the_flag_get_attention_flags_returned` |
| `list_saved_views` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface` |
| `list_tags` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface` |
| `list_custom_field_defs` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface` |
| `get_record_metadata` | "Initial tools → Read" | `the_read_tools_answer_for_every_record_and_metadata_surface` |
| `list_attachments` | "Initial tools → Read" | `attachment_reads_carry_no_file_contents_or_internal_paths` |
| `attachment_path` | "Initial tools → Read" | `attachment_reads_carry_no_file_contents_or_internal_paths` |
| `get_followup_templates` | "Initial tools → Propose" | `the_read_tools_answer_for_every_record_and_metadata_surface` |
| `preview_context` | "Agent onboarding" | `preview_context_shows_what_would_be_sent_without_calling_the_provider`, `preview_context_covers_propose_update_and_refuses_an_unpreviewable_tool` |

### AI-backed tools (a provider call the client asked for)

| Tool | Documented at | Tested at |
| --- | --- | --- |
| `summarize_history` | "Initial tools → Propose" | `summarize_history_calls_the_provider_only_when_the_tool_is_invoked`, `the_assistant_being_off_is_provider_unavailable` |
| `explain_attention_flag` | "Initial tools → Propose" | `explain_attention_flag_answers_the_flag_get_attention_flags_returned`; `tests/attention_explanations.rs::explaining_a_stale_lead_flag_round_trips_through_the_provider` |
| `propose_record` | "Initial tools → Propose" | `a_draft_can_be_proposed_applied_and_undone_over_mcp`; `tests/proposals.rs::a_drafted_contact_applies_through_the_ordinary_create_path` |
| `propose_update` | "Initial tools → Propose" | `preview_context_covers_propose_update_and_refuses_an_unpreviewable_tool`; `tests/proposals.rs::a_drafted_opportunity_update_diffs_and_applies_only_the_changed_fields` |
| `propose_followup` | "Initial tools → Propose" | `propose_followup_drafts_from_a_template_and_applies_as_a_task`; `tests/followups.rs::a_drafted_follow_up_writes_nothing_until_it_is_applied_and_can_be_undone` |

### Write tools (read-write connections only)

| Tool | Documented at | Tested at |
| --- | --- | --- |
| `apply_proposal` | "Initial tools → Write" | `a_draft_can_be_proposed_applied_and_undone_over_mcp`, `an_unknown_draft_surfaces_proposal_expired`, `propose_followup_drafts_from_a_template_and_applies_as_a_task` |
| `undo_proposal` | "Initial tools → Write" | `a_draft_can_be_proposed_applied_and_undone_over_mcp`; `tests/proposals.rs::undoing_an_update_restores_the_stored_before_image` |
| `create_contact` | "Initial tools → Write" | `writes_are_logged_against_the_agent_actor_and_the_client_name`, `record_rules_and_the_lost_reason_rule_carry_their_own_error_kinds` |
| `update_contact` | "Initial tools → Write" | `a_stale_expected_version_surfaces_the_version_conflict_payload` |
| `create_company` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path` |
| `update_company` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path`, `every_version_checked_write_reports_the_conflict_over_mcp` |
| `create_opportunity` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path` |
| `update_opportunity` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path`, `every_version_checked_write_reports_the_conflict_over_mcp` |
| `move_opportunity_stage` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path`, `record_rules_and_the_lost_reason_rule_carry_their_own_error_kinds` |
| `log_activity` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path` |
| `create_task` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path` |
| `complete_task` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path`, `every_version_checked_write_reports_the_conflict_over_mcp` |
| `link_quote` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path`, `every_version_checked_write_reports_the_conflict_over_mcp` |
| `link_job` | "Initial tools → Write" | `every_write_tool_round_trips_through_the_ordinary_application_path`, `every_version_checked_write_reports_the_conflict_over_mcp` |

### Mode, protocol, and audit behavior

| Behavior | Documented at | Tested at |
| --- | --- | --- |
| Handshake reports product and local API versions | "Agent onboarding", "Versioning" | `initialize_reports_the_product_and_local_api_versions` |
| Older MCP revisions accepted, unknown ones fall back | "Agent onboarding" | `an_unsupported_protocol_revision_falls_back_to_ours` |
| Write tools unlisted read-only | "Agent onboarding" | `read_only_mode_lists_no_write_tools`, `mcp.rs::read_only_mode_hides_every_write_tool` (unit) |
| Desktop-only surfaces omitted from v1 tools | "Agent onboarding" (omitted list) | `v1_omits_the_archive_csv_and_backup_tools` |
| Unknown tool / unknown method are JSON-RPC errors | "Agent onboarding" | `an_unknown_tool_is_a_json_rpc_error`, `an_unknown_method_is_a_json_rpc_error_and_notifications_get_no_reply` |
| Every write logs the agent actor and the client name | "Agent onboarding", "Context and privacy" | `writes_are_logged_against_the_agent_actor_and_the_client_name` |
| Stdio transport, graceful shutdown, read-only default binary | "Agent onboarding" | `the_shipped_binary_serves_a_handshake_and_a_read_over_stdio` |
| Missing or newer-schema database refused | "Agent onboarding" | `the_binary_refuses_a_missing_database` |

## Size limits

| Limit | Value | Documented at | Tested at |
| --- | --- | --- | --- |
| Search results per call | 50 | LOCAL_API.md "Agent onboarding", tool description | `mcp.rs::search_never_returns_more_than_fifty_rows` |
| Timeline entries per call | 200 (`MAX_TIMELINE_ENTRIES`) | LOCAL_API.md "Agent onboarding" | `mcp.rs::a_timeline_is_capped_and_its_bodies_truncated` |
| Timeline body characters | 500 (`MAX_TIMELINE_BODY_CHARS`) | LOCAL_API.md "Agent onboarding" | `mcp.rs::a_timeline_is_capped_and_its_bodies_truncated`, `src/mcp.rs::a_long_activity_body_is_truncated_with_a_marker` (unit) |
| List tool `limit` | 1–500 (`MAX_LIST_LIMIT`) | LOCAL_API.md "Agent onboarding" | `mcp.rs::list_tools_take_a_limit_and_refuse_an_unusable_one` |
| Proposal description / update request | 2000 chars | LOCAL_API.md "Initial tools → Propose" | `src/proposals.rs::a_blank_or_over_long_description_is_refused_before_the_model_is_asked` (unit) |
| Proposal warnings per draft | 12 (`MAX_WARNINGS`) | LOCAL_API.md "Initial tools → Propose" | `src/proposals.rs::a_chatty_answer_never_returns_more_than_the_warning_cap` (unit) |
| Proposal context value characters | 200 (`MAX_PROJECTION_VALUE_CHARS`) | LOCAL_API.md "Initial tools → Propose" | `src/proposals.rs::the_context_projection_is_bounded_and_leaves_out_empty_fields` (unit) |
| History projection entries | 25 | LOCAL_API.md "Initial tools → Propose" | `tests/followups.rs::the_summary_projection_is_bounded_and_carries_only_the_target_record` |
| History window days | default 90, max 3650 | LOCAL_API.md "Initial tools → Propose" | `src/followups.rs::the_window_is_bounded` (unit), `tests/followups.rs::a_shorter_window_keeps_older_entries_out_of_the_projection` |
| Suggested next actions | 5, 200 chars each | LOCAL_API.md "Initial tools → Propose" | `src/followups.rs::a_summary_answer_splits_into_a_recap_and_bounded_actions` (unit) |
| Follow-up objective | 500 chars | LOCAL_API.md "Initial tools → Propose" | `src/followups.rs::an_objective_is_trimmed_and_bounded` (unit) |
| Follow-up templates | 20 templates, 80-char names, 2000-char bodies | LOCAL_API.md "Initial tools → Propose" | `src/followups.rs::templates_are_capped_trimmed_and_uniquely_identified` (unit), `tests/followups.rs::template_writes_are_validated_and_capped` |
| Provider request timeout | default 60s, clamped 1–300s | ARCHITECTURE.md "AI rules" | `src/ai.rs::a_request_timeout_is_clamped_into_a_usable_range` (unit) |
| Models offered by a connection test | 50 (`MAX_LISTED_MODELS`) | ARCHITECTURE.md "AI rules" | `tests/ai_provider.rs::the_connection_test_never_lists_more_than_fifty_models` |
| Attachment file size | 256 MiB | LOCAL_API.md `add_attachment` | `tests/attachments.rs::a_file_past_the_size_cap_is_refused_before_it_is_copied` |
| Archive entry / total uncompressed | 256 MiB / ~1 GiB | LOCAL_API.md `preview_archive_import`, `export_archive` | `tests/portable_archive.rs` (`validation_failed` / `archive_too_large` cases) |
| Saved views, tags, field defs, options | 50 / 100 / 50 / 50 | DATA_MODEL.md "`saved_views`", "`tags` and `record_tags`" | `tests/saved_views.rs::saved_view_validation_conflicts_and_limits_are_honest`, `tests/tags_custom_fields.rs::lifecycle_validation_and_audit_failure_roll_back` |
| Navigation recents | 12 | DATA_MODEL.md "`recents` and needs-attention" | `tests/search.rs::navigation_recents_are_persisted_deduplicated_capped_and_skip_inactive_records` |

## Error kinds

The published list is `schemas/v1/local-api.json` `errorKinds`, checked against
`ApplicationError::kind()` by
`tests/schema_contracts.rs::local_api_v1_matches_the_registered_command_contract`.

| Kind | Documented at | Tested at (MCP wire shape / application layer) |
| --- | --- | --- |
| `invalid_input` | LOCAL_API.md "Error contract" | `mcp.rs::malformed_arguments_map_to_invalid_input` |
| `not_found` | LOCAL_API.md "Error contract" | `mcp.rs::a_missing_record_maps_to_not_found` |
| `validation_failed` | LOCAL_API.md "Error contract" | `mcp.rs::record_rules_and_the_lost_reason_rule_carry_their_own_error_kinds` |
| `version_conflict` | LOCAL_API.md "Error contract" | `mcp.rs::a_stale_expected_version_surfaces_the_version_conflict_payload`, `mcp.rs::every_version_checked_write_reports_the_conflict_over_mcp` |
| `missing_lost_reason` | LOCAL_API.md "Error contract" | `mcp.rs::record_rules_and_the_lost_reason_rule_carry_their_own_error_kinds`, `tests/pipeline.rs::lost_move_without_reason_fails_with_missing_lost_reason` |
| `read_only` | LOCAL_API.md "Error contract", "Agent onboarding" | `mcp.rs::a_write_tool_on_a_read_only_connection_is_refused_by_name` |
| `proposal_expired` | LOCAL_API.md "Error contract", "Initial tools → Propose" | `mcp.rs::an_unknown_draft_surfaces_proposal_expired`, `tests/proposals.rs::an_expired_draft_cannot_be_applied` |
| `provider_unavailable` | LOCAL_API.md "Error contract" | `mcp.rs::the_assistant_being_off_is_provider_unavailable`, `tests/ai_provider.rs::a_refused_connection_reports_provider_unavailable` |
| `invalid_stored_data` | LOCAL_API.md "Error contract" | `tests/saved_views.rs::stored_legacy_malformed_and_future_definitions_never_rewrite` |
| `backup_failed` | LOCAL_API.md "Error contract" | `tests/backup.rs::backup_refuses_to_overwrite_without_the_flag` |
| `restore_invalid` | LOCAL_API.md "Error contract" | `tests/backup.rs::restore_rejects_a_corrupted_file_and_leaves_the_live_database_untouched`, `tests/backup.rs::restore_rejects_a_backup_with_a_newer_schema_version` |
| `storage_unavailable` | LOCAL_API.md "Error contract" | `tests/tags_custom_fields.rs::lifecycle_validation_and_audit_failure_roll_back` |
| `io` | LOCAL_API.md "Error contract" | `tests/schema_contracts.rs::local_api_v1_matches_the_registered_command_contract` (kind is published and mapped); raised by filesystem failures in the attachment and archive paths |

## Version-conflict paths

Every version-checked command re-reads the record inside the write transaction
and compares versions there, so the check cannot be raced by another writer.

| Path | Documented at | Tested at |
| --- | --- | --- |
| `update_contact` | LOCAL_API.md "Initial tools → Write" | `tests/companies_contacts.rs::update_contact_rejects_stale_versions`, `mcp.rs::a_stale_expected_version_surfaces_the_version_conflict_payload` |
| `update_company` | LOCAL_API.md "Initial tools → Write" | `tests/companies_contacts.rs::update_company_bumps_version_and_rejects_stale_versions`, `mcp.rs::every_version_checked_write_reports_the_conflict_over_mcp` |
| `update_opportunity` / `move_opportunity_stage` | LOCAL_API.md "Initial tools → Write" | `tests/pipeline.rs::stale_move_is_rejected_with_version_conflict`, `mcp.rs::every_version_checked_write_reports_the_conflict_over_mcp` |
| `complete_task` and every task mutation | LOCAL_API.md "Initial tools → Write" | `tests/tasks.rs::stale_versions_conflict_on_every_mutation`, `mcp.rs::every_version_checked_write_reports_the_conflict_over_mcp` |
| `link_quote` / `link_job` | LOCAL_API.md "Initial tools → Write", HANDOFF.md | `tests/handoff.rs::stale_version_on_link_quote_is_a_version_conflict`, `mcp.rs::every_version_checked_write_reports_the_conflict_over_mcp` |
| Activity update | LOCAL_API.md "Initial tools → Write" | `tests/activities.rs::update_bumps_the_version_and_stale_versions_conflict` |
| Attachment removal | LOCAL_API.md `remove_attachment` | `tests/attachments.rs::attachments_are_copied_under_management_listed_and_removed` |
| Saved views, tags, custom fields | LOCAL_API.md "Initial tools → Write" | `tests/saved_views.rs::saved_view_validation_conflicts_and_limits_are_honest`, `tests/tags_custom_fields.rs::invalid_metadata_and_stale_saved_view_references_are_atomic` |
| `propose_update` (before the model is asked) | LOCAL_API.md "Initial tools → Propose" | `tests/proposals.rs::proposing_an_update_against_a_stale_version_conflicts_before_the_model_is_asked`, `mcp.rs::preview_context_covers_propose_update_and_refuses_an_unpreviewable_tool` |
| `apply_proposal` (asserted versions and the version the draft saw) | LOCAL_API.md "Initial tools → Write" | `tests/proposals.rs::a_stale_expected_version_conflicts_and_keeps_the_draft_usable`, `tests/proposals.rs::applying_without_asserted_versions_still_checks_the_version_the_draft_saw` |
| `undo_proposal` (post-apply version pinning) | LOCAL_API.md "Initial tools → Write" | `tests/proposals.rs::undo_refuses_when_the_record_moved_since_it_was_applied`, `tests/proposals.rs::undo_never_silently_reverts_over_work_done_after_the_apply` |

## Known gap

`move_opportunity_stage` needs a `toStageId` (and a `lostReasonId` when the
target is a lost stage), but the v1 tool surface has no way to list stages or
lost reasons — `list_stages` and `list_lost_reasons` exist as application
commands and are not exposed as MCP tools. An agent can only learn a stage id
from an opportunity it already read, so it cannot move work forward on its own.
Exposing those two read tools is a small feature, not a test gap, and is left
for a follow-up item.
