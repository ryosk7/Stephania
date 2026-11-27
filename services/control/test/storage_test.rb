# frozen_string_literal: true

require_relative "test_helper"

class StorageTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "a" * 32
  SCENE = {"version" => 1, "name" => "LT", "layers" => []}.freeze

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.token = TOKEN
    header "Host", "localhost"
    header "Authorization", "Bearer #{TOKEN}"
    header "Content-Type", "application/json"
  end

  def body = JSON.parse(last_response.body)

  def save(id, revision, name = "LT")
    put "/api/v1/scenes/#{id}", JSON.generate(revision: revision, document: SCENE.merge("name" => name))
    assert_equal 200, last_response.status, last_response.body
  end

  def test_migrations_are_recorded_and_idempotent
    Dir.mktmpdir do |dir|
      path = File.join(dir, "control.db")
      Control.configure_storage(path)
      Control.configure_storage(path)
      assert_equal 2, Control.db[:schema_info].get(:version)
      assert Control.db.table_exists?(:scene_revisions)
    end
  end

  def test_adopts_pre_migration_foundation_database
    Dir.mktmpdir do |dir|
      path = File.join(dir, "legacy.db")
      legacy = Sequel.sqlite(path)
      legacy.create_table(:scenes) do
        String :id, primary_key: true
        Integer :revision, null: false
        String :document, text: true, null: false
      end
      legacy[:scenes].insert(id: "lt", revision: 3, document: JSON.generate(SCENE))
      legacy.disconnect
      Control.configure_storage(path)
      get "/api/v1/scenes/lt/revisions"
      assert_equal [3], body.map { |r| r["revision"] }
      save "lt", 3
      get "/api/v1/scenes/lt"
      assert_equal 4, body["revision"]
    end
  end

  def test_revision_history_is_bounded
    (0...(Control::MAX_REVISIONS_KEPT + 5)).each { |r| save "lt", r, "n#{r}" }
    get "/api/v1/scenes/lt/revisions"
    revisions = body.map { |r| r["revision"] }
    assert_equal Control::MAX_REVISIONS_KEPT, revisions.length
    assert_equal Control::MAX_REVISIONS_KEPT + 5, revisions.first
    assert_equal "n#{Control::MAX_REVISIONS_KEPT + 4}", body.first.dig("document", "name")
  end

  def test_delete_requires_current_revision_and_cascades
    save "lt", 0
    delete "/api/v1/scenes/lt"
    assert_equal 422, last_response.status
    delete "/api/v1/scenes/lt?revision=9"
    assert_equal 409, last_response.status
    delete "/api/v1/scenes/lt?revision=1"
    assert_equal 204, last_response.status
    assert_equal 0, Control.db[:scene_revisions].count
    get "/api/v1/scenes/lt"
    assert_equal 404, last_response.status
  end
end

class RestartReconciliationTest < Minitest::Test
  include Rack::Test::Methods

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.token = "a" * 32
    @instance = "aaaaaaaaaaaaaaaa"
    @engine = FakeEngine.new { |req| FakeEngine.ok(req, {"instance_id" => @instance, "state" => "idle"}) }
    Control.engine = EngineClient.new(socket_path: @engine.path)
    header "Host", "localhost"
    header "Authorization", "Bearer #{"a" * 32}"
  end

  def teardown = @engine.close

  def restarted?
    get "/api/v1/status"
    assert_equal 200, last_response.status
    JSON.parse(last_response.body)["engine_restarted"]
  end

  def test_detects_engine_instance_change
    assert_equal false, restarted?
    assert_equal false, restarted?
    @instance = "bbbbbbbbbbbbbbbb"
    assert_equal true, restarted?
    assert_equal false, restarted?
  end
end
