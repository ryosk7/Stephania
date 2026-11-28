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
    header "Host", "stephania.local"
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
    use_engine { |req| FakeEngine.error(req, "engine_start_failed", "STEPHANIA_OUTPUT is not configured") }
    auth
    post "/api/v1/stream/start"
    assert_error 503, "engine_start_failed"
    assert_equal "STEPHANIA_OUTPUT is not configured", body.dig("error", "message")
    use_engine { |req| FakeEngine.error(req, "something_new") }
    post "/api/v1/stream/start"
    assert_error 502, "something_new"
  end

  def test_engine_missing_is_503
    @engine.close
    @engine = nil
    Control.engine = EngineClient.new(socket_path: File.join(Dir.tmpdir, "stephania-missing-#{Process.pid}.sock"))
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

  # Checked before Sinatra parses anything and before authentication, so an
  # anonymous client cannot make control parse large or multipart bodies.
  def test_body_limits_apply_before_parsing_and_authentication
    header "Authorization", nil
    header "Content-Type", "application/x-www-form-urlencoded"
    post "/api/v1/auth/session", "a=#{"x" * Control::MAX_BODY_BYTES}"
    assert_error 413, "body_too_large"
    header "Content-Type", "multipart/form-data; boundary=x"
    post "/api/v1/assets", "--x\r\nContent-Disposition: form-data; name=\"f\"; filename=\"a\"\r\n\r\nA\r\n--x--\r\n"
    assert_error 415, "unsupported_media_type"
    header "Content-Type", "image/png"
    post "/api/v1/assets", "x" * (Control::MAX_BODY_BYTES + 1)
    assert_error 401, "unauthorized" # the upload route has its own, larger limit
    post "/api/v1/%61ssets", "x" * (Control::MAX_BODY_BYTES + 1)
    assert_error 413, "body_too_large"
    post "/api/v1/assets", "x" * (AssetStore::MAX_BYTES + 1)
    assert_error 413, "body_too_large"
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

  # A scene that passes every per-field limit can still be too large for
  # one engine IPC frame; it is refused at validation (422), not at apply (500).
  def test_scene_that_cannot_fit_an_ipc_frame_is_invalid
    layer = lambda do |i, chars|
      vars = (0...16).to_h { |k| ["k#{k}", "\u00e9" * chars] }
      {"id" => "l#{i}", "type" => "image", "x" => 0, "y" => 0, "width" => 10, "height" => 10,
       "visible" => true, "asset" => "0123456789abcdef", "template" => {"id" => "t", "vars" => vars}}
    end
    doc = ->(chars) { {"version" => 2, "name" => "big", "layers" => (0...16).map { |i| layer.call(i, chars) }} }
    assert_nil SceneValidator.validate(doc.call(20))
    assert_equal ["", "document_too_large"], SceneValidator.validate(doc.call(200))
  end

  def test_invalid_fixtures
    reasons = JSON.parse(File.read(File.join(FIXTURES, "scenes", "invalid-reasons.json")))
    fixtures("invalid").each do |name, doc|
      result = SceneValidator.validate(doc)
      refute_nil result, name
      assert_equal reasons.fetch(name), result[1], name
    end
  end
end

class CameraSessionTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "c" * 32

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.token = TOKEN
    @phone = FakeEngine.new do |req|
      case req["method"]
      when "session.offer"
        if req.dig("params", "sdp") == "busy" then FakeEngine.error(req, "busy", "a phone session is active")
        else FakeEngine.ok(req, {"sdp" => "v=0 answer"})
        end
      when "session.status" then FakeEngine.ok(req, {"state" => "connected"})
      when "session.close" then FakeEngine.ok(req, {})
      end
    end
    Control.phone = EngineClient.new(socket_path: @phone.path)
    header "Host", "localhost"
    header "Authorization", "Bearer #{TOKEN}"
    header "Content-Type", "application/json"
  end

  def teardown = @phone.close

  def test_relays_offer_and_maps_errors
    post "/api/v1/cameras/session/offer", JSON.generate(sdp: "v=0 offer", tracks: {"t1" => "rear"})
    assert_equal 200, last_response.status, last_response.body
    assert_equal "v=0 answer", JSON.parse(last_response.body)["sdp"]
    req = @phone.requests.pop
    assert_equal({"sdp" => "v=0 offer", "tracks" => {"t1" => "rear"}}, req["params"])
    post "/api/v1/cameras/session/offer", JSON.generate(sdp: "busy", tracks: {"t1" => "rear"})
    assert_equal 409, last_response.status
    assert_equal "busy", JSON.parse(last_response.body).dig("error", "code")
    post "/api/v1/cameras/session/offer", JSON.generate(sdp: "x", tracks: {}, extra: 1)
    assert_equal 422, last_response.status
  end

  def test_status_close_and_unavailable
    get "/api/v1/cameras/session"
    assert_equal "connected", JSON.parse(last_response.body)["state"]
    delete "/api/v1/cameras/session"
    assert_equal 204, last_response.status
    Control.phone = EngineClient.new(socket_path: File.join(Dir.tmpdir, "stephania-no-phone-#{Process.pid}.sock"))
    get "/api/v1/cameras/session"
    assert_equal 503, last_response.status
    assert_equal "phone_unavailable", JSON.parse(last_response.body).dig("error", "code")
  end

  def test_requires_token
    header "Authorization", "Bearer #{"x" * 32}"
    post "/api/v1/cameras/session/offer", JSON.generate(sdp: "v=0", tracks: {"t" => "rear"})
    assert_equal 401, last_response.status
  end
end

class CameraCredentialTest < Minitest::Test
  include Rack::Test::Methods

  OWNER = "o" * 32

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.token = OWNER
    Control.phone_session_owner = nil
    @phone = FakeEngine.new do |req|
      case req["method"]
      when "session.offer" then FakeEngine.ok(req, {"sdp" => "answer"})
      else FakeEngine.ok(req, {"state" => "none"})
      end
    end
    Control.phone = EngineClient.new(socket_path: @phone.path)
    @engine = FakeEngine.new { |req| FakeEngine.ok(req, {"state" => "idle"}) }
    Control.engine = EngineClient.new(socket_path: @engine.path)
    header "Host", "localhost"
    header "Content-Type", "application/json"
  end

  def teardown
    @phone.close
    @engine.close
  end

  def as(token) = header("Authorization", "Bearer #{token}")
  def body = JSON.parse(last_response.body)

  def issue(ttl: 3600, label: "stage phone")
    as OWNER
    post "/api/v1/credentials/camera", JSON.generate(ttl_s: ttl, label: label)
    assert_equal 201, last_response.status, last_response.body
    body
  end

  def offer(replace: nil)
    payload = {sdp: "v=0", tracks: {"t" => "rear"}}
    payload[:replace] = replace unless replace.nil?
    post "/api/v1/cameras/session/offer", JSON.generate(payload)
  end

  def test_camera_token_only_reaches_the_camera_session
    cred = issue
    assert_match(/\At3c_[0-9a-f]{64}\z/, cred["token"])
    as cred["token"]
    offer
    assert_equal 200, last_response.status, last_response.body
    get "/api/v1/cameras/session"
    assert_equal 200, last_response.status
    %w[/api/v1/status /api/v1/scenes /api/v1/credentials/camera /api/v1/events].each do |path|
      get path
      assert_equal 403, last_response.status, path
      assert_equal "forbidden_role", body.dig("error", "code")
    end
    post "/api/v1/stream/start"
    assert_equal 403, last_response.status
    post "/api/v1/credentials/camera", JSON.generate(ttl_s: 60, label: "x")
    assert_equal 403, last_response.status, "a camera cannot mint credentials"
  end

  def test_only_the_hash_is_stored_and_listing_hides_tokens
    cred = issue
    stored = Control.db[:camera_credentials].first
    refute_includes stored.values.map(&:to_s), cred["token"]
    as OWNER
    get "/api/v1/credentials/camera"
    assert_equal [cred["id"]], body.map { |c| c["id"] }
    refute body.first.key?("token")
  end

  def test_expiry_and_revocation
    cred = issue(ttl: 60)
    Control.db[:camera_credentials].update(expires_at_unix: Time.now.to_i - 1)
    as cred["token"]
    get "/api/v1/cameras/session"
    assert_equal 401, last_response.status, "expired"

    cred = issue
    as cred["token"]
    offer
    assert_equal cred["id"], Control.phone_session_owner
    as OWNER
    delete "/api/v1/credentials/camera/#{cred["id"]}"
    assert_equal 204, last_response.status
    assert_nil Control.phone_session_owner
    closes = []
    closes << @phone.requests.pop until @phone.requests.empty?
    assert_includes closes.map { |r| r["method"] }, "session.close", "revoking ends that camera's session"
    as cred["token"]
    get "/api/v1/cameras/session"
    assert_equal 401, last_response.status
  end

  def test_a_camera_cannot_take_over_or_end_another_session
    first, second = issue, issue
    as OWNER
    offer
    as second["token"]
    offer(replace: true)
    assert_equal 403, last_response.status
    delete "/api/v1/cameras/session"
    assert_equal 403, last_response.status
    as first["token"]
    delete "/api/v1/cameras/session"
    assert_equal 403, last_response.status
    as OWNER
    delete "/api/v1/cameras/session"
    assert_equal 204, last_response.status
  end

  def test_validation_and_bounds
    as OWNER
    post "/api/v1/credentials/camera", JSON.generate(ttl_s: 10, label: "x")
    assert_equal 422, last_response.status
    post "/api/v1/credentials/camera", JSON.generate(ttl_s: 100_000)
    assert_equal 422, last_response.status
    Control::MAX_CAMERA_CREDENTIALS.times { issue }
    as OWNER
    post "/api/v1/credentials/camera", JSON.generate(ttl_s: 60, label: "x")
    assert_equal 409, last_response.status
    delete "/api/v1/credentials/camera/zzzz"
    assert_equal 400, last_response.status
    delete "/api/v1/credentials/camera/00000000"
    assert_equal 404, last_response.status
  end
end
