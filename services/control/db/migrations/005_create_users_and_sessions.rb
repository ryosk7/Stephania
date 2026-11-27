# frozen_string_literal: true

# Studio accounts (docs/AUTH.md). Passwords are scrypt hashes; sessions are
# stored only as the SHA-256 of the cookie value.
Sequel.migration do
  up do
    create_table(:users) do
      primary_key :id
      String :username, null: false, unique: true
      String :password_hash, null: false
      String :role, null: false # admin | operator
      TrueClass :disabled, null: false, default: false
      Integer :created_at_unix, null: false
    end
    create_table(:sessions) do
      String :token_sha256, primary_key: true
      foreign_key :user_id, :users, null: false, on_delete: :cascade
      String :csrf_token, null: false
      Integer :created_at_unix, null: false
      Integer :last_seen_unix, null: false
      Integer :expires_at_unix, null: false
    end
  end

  down do
    drop_table(:sessions)
    drop_table(:users)
  end
end
