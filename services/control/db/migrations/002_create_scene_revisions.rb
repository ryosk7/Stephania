# frozen_string_literal: true

# Bounded draft history (docs/MASTER-ARCHITECTURE.md §20). Existing scenes are
# backfilled with their current revision so history is never empty.
Sequel.migration do
  up do
    create_table(:scene_revisions) do
      String :scene_id, null: false
      Integer :revision, null: false
      String :document, text: true, null: false
      Integer :created_at_unix, null: false
      primary_key %i[scene_id revision]
      foreign_key [:scene_id], :scenes, key: [:id], on_delete: :cascade
    end
    now = Time.now.to_i
    from(:scenes).each do |row|
      from(:scene_revisions).insert(scene_id: row[:id], revision: row[:revision], document: row[:document], created_at_unix: now)
    end
  end

  down do
    drop_table(:scene_revisions)
  end
end
