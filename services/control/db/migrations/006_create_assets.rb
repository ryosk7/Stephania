# frozen_string_literal: true

# Image assets for scene v2 image layers (docs/SCENE.md). Files live in
# STEPHANIA_ASSETS_DIR as <id>.png, id = first 16 hex digits of their SHA-256.
Sequel.migration do
  up do
    create_table(:assets) do
      String :id, primary_key: true
      String :sha256, null: false
      String :name, null: false
      Integer :width, null: false
      Integer :height, null: false
      Integer :bytes, null: false
      Integer :created_at_unix, null: false
    end
  end

  down do
    drop_table(:assets)
  end
end
