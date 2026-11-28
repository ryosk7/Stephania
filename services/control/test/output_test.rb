# frozen_string_literal: true

require_relative "test_helper"

# Stream destination API (/api/v1/output) and its use by stream.start.
class OutputTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "q" * 32
  PASSWORD = "correct horse battery"
  KEY = "abcd-efgh-ijkl-mnop-wxyz"
  URL = "rtmps://a.rtmps.youtube.com/live2"

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.token = TOKEN
    Control.throttle = Accounts::Throttle.new
    @dir = Dir.mktmpdir("stephania-output-api")
    Control.stream_output = OutputStore.new(File.join(@dir, "output.json"))
    @engine = FakeEngine.new { |req| FakeEngine.ok(req, {"accepted" => true}) }
    Control.engine = EngineClient.new(socket_path: @engine.path)
    header "Host", "localhost"
    header "Content-Type", "application/json"
  end

  def teardown
    @engine.close
    Control.stream_output = OutputStore.new(nil)
    FileUtils.remove_entry(@dir)
  end

  def body = JSON.parse(last_response.body)
  def as_owner = header("Authorization", "Bearer #{TOKEN}")

  def set_output(url = URL, key = KEY)
    put "/api/v1/output", JSON.generate(url: url, stream_key: key)
  end

  def login_as(name, role)
    as_owner
    post "/api/v1/users", JSON.generate(username: name, password: PASSWORD, role: role)
    assert_equal 201, last_response.status, last_response.body
    header "Authorization", nil
    post "/api/v1/auth/session", JSON.generate(username: name, password: PASSWORD)
    assert_equal 201, last_response.status, last_response.body
    header "X-CSRF-Token", body["csrf_token"]
  end

  def test_admin_sets_replaces_and_clears_without_the_key_ever_returned
    as_owner
    get "/api/v1/output"
    assert_equal({"configured" => false, "url" => nil, "key_last4" => nil, "updated_at_unix" => nil}, body)
    set_output
    assert_equal 200, last_response.status, last_response.body
    assert_equal true, body["configured"]
    assert_equal URL, body["url"]
    assert_equal "wxyz", body["key_last4"]
    refute_includes last_response.body, KEY
    get "/api/v1/output"
    refute_includes last_response.body, "mnop"
    assert_equal 0o600, File.stat(Control.stream_output.path).mode & 0o777
    set_output("rtmps://b.rtmps.youtube.com/live2", "zzzz-yyyy-xxxx-1234")
    assert_equal "1234", body["key_last4"]
    delete "/api/v1/output"
    assert_equal 204, last_response.status
    get "/api/v1/output"
    assert_equal false, body["configured"]
  end

  def test_stream_start_sends_the_stored_output_only_over_ipc
    as_owner
    post "/api/v1/stream/start"
    assert_equal 202, last_response.status, last_response.body
    refute @engine.requests.pop.key?("params"), "nothing stored: the engine falls back to STEPHANIA_OUTPUT"
    set_output
    post "/api/v1/stream/start"
    assert_equal 202, last_response.status, last_response.body
    refute_includes last_response.body, KEY
    assert_equal({"output" => "#{URL}/#{KEY}"}, @engine.requests.pop["params"])
  end

  def test_validation_errors_do_not_echo_the_key
    as_owner
    set_output("rtmp://a.rtmp.youtube.com/live2", KEY)
    assert_equal 422, last_response.status
    assert_equal "invalid_output", body.dig("error", "code")
    refute_includes last_response.body, KEY
    set_output(URL, "bad key with spaces")
    assert_equal 422, last_response.status
    refute_includes last_response.body, "spaces"
    put "/api/v1/output", JSON.generate(url: URL)
    assert_equal 422, last_response.status
    assert_equal "invalid_request", body.dig("error", "code")
    get "/api/v1/output"
    assert_equal false, body["configured"]
  end

  def test_operator_sees_only_whether_an_output_is_configured
    as_owner
    set_output
    login_as("olga", "operator")
    get "/api/v1/output"
    assert_equal 200, last_response.status, last_response.body
    assert_equal({"configured" => true}, body)
    set_output(URL, "operator-replaced-key")
    assert_equal 403, last_response.status
    assert_equal "forbidden_role", body.dig("error", "code")
    delete "/api/v1/output"
    assert_equal 403, last_response.status
    assert_equal "#{URL}/#{KEY}", Control.stream_output.output_url
  end

  def test_signed_in_admin_needs_csrf
    login_as("alice", "admin")
    header "X-CSRF-Token", nil
    set_output
    assert_equal 403, last_response.status
    assert_equal "csrf_failed", body.dig("error", "code")
  end

  def test_requires_authentication
    get "/api/v1/output"
    assert_equal 401, last_response.status
  end
end
