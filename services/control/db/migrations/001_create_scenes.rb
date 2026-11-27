# frozen_string_literal: true

# create_table? adopts databases created by the pre-migration foundation
# build, which created this exact table without schema_info.
Sequel.migration do
  up do
    create_table?(:scenes) do
      String :id, primary_key: true
      Integer :revision, null: false
      String :document, text: true, null: false
    end
  end

  down do
    drop_table?(:scenes)
  end
end
