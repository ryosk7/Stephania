# frozen_string_literal: true

# Camera-role credentials (docs/MASTER-ARCHITECTURE.md §26: Owner, Operator,
# Camera). Only SHA-256 of the token is stored; tokens expire.
Sequel.migration do
  up do
    create_table(:camera_credentials) do
      String :id, primary_key: true
      String :token_sha256, null: false, unique: true
      String :label, null: false
      Integer :created_at_unix, null: false
      Integer :expires_at_unix, null: false
    end
  end

  down do
    drop_table(:camera_credentials)
  end
end
