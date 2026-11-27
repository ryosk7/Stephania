# frozen_string_literal: true

require_relative "test_helper"

class ControlTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "a" * 32
  SCENE = {version: 1, name: "LT", layers: [{id: "hdmi", source: "hdmi", x: 0, y: 0, width: 1920, height: 1080, visible: true}]}.freeze

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.token = TOKEN
    @engine = nil
    use_engine { |req| FakeEngine.ok(req, {"method" => req["method"], "params" => req["params"]}) }
    header "Host", "localhost"
  end

  def teardown
    @engine&.close
  end

  def use_engine(timeout: 2.0, &handler)
    @engine&.close
    @engine = FakeEngine.new(&handler)
    Control.engine = EngineClient.new(socket_path: @engine.path, timeout: timeout)
  end

  def auth = header("Authorization", "Bearer #{TOKEN}")
  def body = JSON.parse(last_response.body)

  def assert_error(status, code)
    assert_equal status, last_response.status, last_response.body
    assert_equal code, body.dig("error", "code")
    assert_equal last_response.headers["X-Request-Id"], body.dig("error", "request_id")
  end

  def put_scene(id, revision, document)
    header "Content-Type", "application/json"
    put "/api/v1/scenes/#{id}", JSON.generate(revision: revision, document: document)
  end

  def test_requires_bearer_token
    get "/api/v1/status"
    assert_error 401, "unauthorized"
    header "Authorization", "Bearer " + ("b" * 32)
    get "/api/v1/status"
    assert_error 401, "unauthorized"
    assert_empty @engine.requests
  end

  def test_fails_closed_without_configured_token
    Control.token = "short"
    auth
    get "/api/v1/status"
    assert_error 503, "not_configured"
  end

  def test_rejects_non_loopback_host
    header "Host", "t3box.local"
    auth
    get "/api/v1/status"
    assert_error 403, "forbidden_host"
  end

  # Regression: the injected engine adapter must be invoked with its
  # arguments (previously Sinatra evaluated it as a zero-arity setting → 500).
  def test_stream_start_reaches_engine_over_ipc
    auth
    post "/api/v1/stream/start"
    assert_equal 202, last_response.status, last_response.body
    assert_equal "stream.start", body["method"]
    request = @engine.requests.pop
    assert_equal 1, request["version"]
    assert_equal last_response.headers["X-Request-Id"], request["request_id"]
    refute request.key?("params")
  end

  def test_status_and_stop_proxy_engine_results
    auth
    get "/api/v1/status"
    assert_equal 200, last_response.status
    assert_equal "engine.status", body["method"]
    post "/api/v1/stream/stop"
    assert_equal "stream.stop", body["method"]
  end

  def test_engine_error_codes_map_to_http
    use_engine { |req| FakeEngine.error(req, "engine_start_failed", "T3BOX_OUTPUT is not configured") }
    auth
    post "/api/v1/stream/start"
    assert_error 503, "engine_start_failed"
    assert_equal "T3BOX_OUTPUT is not configured", body.dig("error", "message")
    use_engine { |req| FakeEngine.error(req, "something_new") }
    post "/api/v1/stream/start"
    assert_error 502, "something_new"
  end

  def test_engine_missing_is_503
    @engine.close
    @engine = nil
    Control.engine = EngineClient.new(socket_path: File.join(Dir.tmpdir, "t3box-missing-#{Process.pid}.sock"))
    auth
    get "/api/v1/status"
    assert_error 503, "engine_unavailable"
  end

  def test_engine_protocol_violations_are_503
    auth
    [
      "not json",
      JSON.generate({"version" => 2, "ok" => true}),
      ->(req) { FakeEngine.ok(req, {}).merge("request_id" => "other") }
    ].each do |reply|
      use_engine { |req| reply.respond_to?(:call) ? JSON.generate(reply.call(req)) : reply }
      get "/api/v1/status"
      assert_error 503, "engine_unavailable"
    end
  end

  def test_hung_engine_times_out
    use_engine(timeout: 0.3) { :hang }
    auth
    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    get "/api/v1/status"
    assert_error 503, "engine_unavailable"
    assert_operator Process.clock_gettime(Process::CLOCK_MONOTONIC) - started, :<, 2
  end

  def test_scene_revision_conflict_and_validation
    auth
    put_scene "lt", 0, SCENE
    assert_equal 200, last_response.status
    assert_equal 1, body["revision"]
    put_scene "lt", 0, SCENE
    assert_error 409, "revision_conflict"
    bad = JSON.parse(JSON.generate(SCENE))
    bad["layers"][0]["width"] = 1921
    put_scene "lt", 1, bad
    assert_error 422, "invalid_scene"
    put_scene "lt", 1, SCENE
    assert_equal 2, body["revision"]
    get "/api/v1/scenes/lt"
    assert_equal 2, body["revision"]
    get "/api/v1/scenes/nope"
    assert_error 404, "not_found"
    get "/api/v1/scenes/bad%20id"
    assert_error 400, "invalid_id"
    get "/api/v1/scenes"
    assert_equal ["lt"], body.map { |s| s["id"] }
  end

  def test_body_limits_and_json_errors
    auth
    header "Content-Type", "application/json"
    put "/api/v1/scenes/lt", "{"
    assert_error 400, "invalid_json"
    put "/api/v1/scenes/lt", "x" * (Control::MAX_BODY_BYTES + 1)
    assert_error 413, "body_too_large"
    put "/api/v1/scenes/lt", JSON.generate(revision: 0, document: SCENE, extra: 1)
    assert_error 422, "invalid_request"
  end

  def test_program_transition_fails_closed_through_engine
    use_engine { |req| FakeEngine.error(req, "not_implemented", "compositor is not implemented; Program unchanged") }
    auth
    put_scene "lt", 0, SCENE
    header "Content-Type", "application/json"
    post "/api/v1/program/transition", JSON.generate(scene_id: "lt", revision: 1)
    assert_error 501, "not_implemented"
    request = @engine.requests.pop
    assert_equal "scene.apply", request["method"]
    assert_equal JSON.parse(JSON.generate(SCENE)), request.dig("params", "scene")
    assert_equal "lt", request.dig("params", "scene_id")
    assert_equal 1, request.dig("params", "revision")
    post "/api/v1/program/transition", JSON.generate(scene_id: "lt", revision: 7)
    assert_error 409, "revision_conflict"
    post "/api/v1/program/transition", JSON.generate(scene_id: "missing", revision: 1)
    assert_error 404, "not_found"
    assert_equal 0, @engine.requests.size
  end

  def test_program_transition_acknowledged_and_inactive
    auth
    put_scene "lt", 0, SCENE
    header "Content-Type", "application/json"
    use_engine { |req| FakeEngine.ok(req, {"applied" => true, "scene_id" => "lt", "revision" => 1, "first_frame_index" => 42}) }
    post "/api/v1/program/transition", JSON.generate(scene_id: "lt", revision: 1)
    assert_equal 200, last_response.status, last_response.body
    assert_equal 42, body.dig("engine", "first_frame_index")
    use_engine { |req| FakeEngine.error(req, "program_inactive", "no stream is running") }
    post "/api/v1/program/transition", JSON.generate(scene_id: "lt", revision: 1)
    assert_error 409, "program_inactive"
  end

  def test_unknown_api_route_uses_envelope
    auth
    get "/api/v1/nope"
    assert_error 404, "not_found"
  end
end

class SceneFixtureConformanceTest < Minitest::Test
  def fixtures(kind)
    files = Dir[File.join(FIXTURES, "scenes", kind, "*.json")].sort
    refute_empty files
    files.map { |f| [File.basename(f), JSON.parse(File.read(f))] }
  end

  def test_valid_fixtures
    fixtures("valid").each { |name, doc| assert_nil SceneValidator.validate(doc), name }
  end

  def test_invalid_fixtures
    fixtures("invalid").each { |name, doc| refute_nil SceneValidator.validate(doc), name }
  end
end
