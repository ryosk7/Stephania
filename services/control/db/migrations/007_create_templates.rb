# frozen_string_literal: true

# Developer overlay templates (docs/SCENE.md "Templates"): SVG with
# {{variable}} placeholders, rendered to PNG assets by the Studio.
Sequel.migration do
  up do
    create_table(:templates) do
      String :id, primary_key: true
      String :name, null: false
      Integer :width, null: false
      Integer :height, null: false
      String :svg, text: true, null: false
      String :vars, text: true, null: false # JSON [{key, label, initial}]
      Integer :updated_at_unix, null: false
    end
  end

  down do
    drop_table(:templates)
  end
end
