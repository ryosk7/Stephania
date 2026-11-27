# frozen_string_literal: true

# Program audio inputs (docs/AUDIO.md). One row per configured input; the
# engine keeps its built-in defaults for inputs without a row.
Sequel.migration do
  up do
    create_table(:audio_inputs) do
      String :id, primary_key: true # hdmi | mic
      TrueClass :enabled, null: false
      String :device, null: false
      Integer :channels, null: false
      Float :gain_db, null: false
      TrueClass :muted, null: false
      Integer :updated_at_unix, null: false
    end
  end

  down do
    drop_table(:audio_inputs)
  end
end
