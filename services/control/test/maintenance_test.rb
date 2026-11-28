# frozen_string_literal: true

require_relative "test_helper"

# /api/v1/maintenance (docs/MAINTENANCE.md): admin only, relayed to
# stephania-maint with the account name and the TCP peer address.
class MaintenanceTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "m" * 32
  ISSUED = "t3b_#{"2e" * 32}"
  PASSWORD = "correct horse battery"
  WINDOW = {"window_id" => "0123456789abcdef", "root" => false, "actor" => "owner", "user" => "maint",
            "opened_at" => 100, "expires_at" => 700, "sessions_end_at" => 3700, "accepting" => true}.freeze

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.token = TOKEN
    Control.throttle = Accounts::Throttle.new
    Control.login_gate = Accounts::LoginGate.new
    @maint = FakeEngine.new do |req|
      p = req["params"] || {}
      case req["method"]
      when "status" then FakeEngine.ok(req, {"open" => false, "window" => nil, "code_locked_until" => nil})
      when "window.open"
        case p["code"]
        when "000000" then FakeEngine.error(req, "invalid_code", "code not accepted")
        when "999999" then FakeEngine.error(req, "code_throttled", "too many wrong codes, retry in 60 s").tap { |r| r["error"]["retry_after"] = 60 }
        when "555555" then FakeEngine.error(req, "internal", "could not start sshd")
        else FakeEngine.ok(req, WINDOW.merge("actor" => p["actor"], "root" => p["root"], "user" => p["root"] ? "root" : "maint"))
        end
      when "cert.sign"
        if p["window_id"] == WINDOW["window_id"]
          FakeEngine.ok(req, {"certificate" => "ssh-ed25519-cert-v01@openssh.com AAAA", "valid_before" => 400, "serial" => 3,
                              "host_ca" => "ssh-ed25519 BBBB stephania-host-ca", "principal" => "maint", "user" => "maint"})
        else FakeEngine.error(req, "window_closed", "no open maintenance window with that id")
        end
      when "window.close" then FakeEngine.ok(req, {"closed" => true})
      else FakeEngine.error(req, "invalid_params", "unknown method")
      end
    end
    Control.maint = EngineClient.new(socket_path: @maint.path)
    header "Host", "localhost"
    header "Content-Type", "application/json"
    as_owner
  end

  def teardown
    @maint.close
    FileUtils.remove_entry(@pki) if @pki
    Control.lan = LanAccess.new(pki_dir: nil, tokens_file: nil)
  end

  # A LAN client: TLS listener, certified Host, enrolled owner token, from
  # REMOTE_ADDR (with a forged X-Forwarded-For that must be ignored).
  def lan_env(addr = "192.168.1.20")
    @pki ||= Dir.mktmpdir("stephania-maint-lan").tap do |dir|
      File.write(File.join(dir, "endpoint.json"), JSON.generate(host: "stephania-m.local", ips: ["192.168.1.9"]))
      File.write(File.join(dir, "tokens.json"), JSON.generate(tokens: [{id: "x", sha256: OpenSSL::Digest::SHA256.hexdigest(ISSUED)}]))
      Control.lan = LanAccess.new(pki_dir: dir, tokens_file: File.join(dir, "tokens.json"))
    end
    header "Authorization", "Bearer #{ISSUED}"
    {"REMOTE_ADDR" => addr, "HTTPS" => "on", "rack.url_scheme" => "https", "HTTP_HOST" => "stephania-m.local"}
  end
  def body = JSON.parse(last_response.body)
  def as_owner = header("Authorization", "Bearer #{TOKEN}")
  def requests = Array.new(@maint.requests.size) { @maint.requests.pop }

  def login_as(name, role)
    as_owner
    post "/api/v1/users", JSON.generate(username: name, password: PASSWORD, role: role)
    assert_equal 201, last_response.status, last_response.body
    header "Authorization", nil
    post "/api/v1/auth/session", JSON.generate(username: name, password: PASSWORD)
    assert_equal 201, last_response.status, last_response.body
    header "X-CSRF-Token", body["csrf_token"]
  end

  def open_window(params = {code: "123456"}, env = {})
    post "/api/v1/maintenance/window", JSON.generate(params), env
  end

  def test_owner_opens_a_window_with_the_peer_address_not_forwarded_headers
    open_window({code: "123456"}, lan_env.merge("HTTP_X_FORWARDED_FOR" => "10.9.9.9"))
    assert_equal 201, last_response.status, last_response.body
    assert_equal({"window_id" => "0123456789abcdef", "expires_at" => 700, "sessions_end_at" => 3700, "root" => false, "user" => "maint"}, body)
    req = requests.last
    assert_equal "window.open", req["method"]
    assert_equal({"actor" => "owner", "client_addr" => "192.168.1.20", "code" => "123456", "root" => false}, req["params"])
  end

  def test_root_window_is_explicit
    open_window(code: "123456", root: true)
    assert_equal 201, last_response.status
    assert_equal "root", body["user"]
    assert_equal true, requests.last.dig("params", "root")
  end

  def test_admin_account_name_is_the_actor_and_csrf_is_required
    login_as("alice", "admin")
    open_window
    assert_equal 201, last_response.status, last_response.body
    assert_equal "alice", requests.last.dig("params", "actor")
    header "X-CSRF-Token", "wrong"
    delete "/api/v1/maintenance/window"
    assert_equal 403, last_response.status
    assert_equal "csrf_failed", body.dig("error", "code")
  end

  def test_operators_can_neither_read_nor_open
    login_as("olga", "operator")
    get "/api/v1/maintenance"
    assert_equal 403, last_response.status
    open_window
    assert_equal 403, last_response.status
    assert_empty requests.select { |r| r["method"] == "window.open" }
  end

  def test_malformed_bodies_never_reach_the_device
    [{code: "12345"}, {code: 123_456}, {code: "123456", extra: 1}, {code: "123456", root: "yes"}, {}].each do |bad|
      open_window(bad)
      assert_equal 422, last_response.status, bad.inspect
    end
    post "/api/v1/maintenance/certificate", JSON.generate(window_id: "../x", public_key: "ssh-ed25519 AAAA")
    assert_equal 422, last_response.status
    assert_empty requests
  end

  def test_device_errors_map_to_http
    open_window(code: "000000")
    assert_equal [401, "invalid_code"], [last_response.status, body.dig("error", "code")]
    open_window(code: "999999")
    assert_equal [429, "code_throttled", "60"], [last_response.status, body.dig("error", "code"), last_response.headers["Retry-After"]]
    open_window(code: "555555")
    assert_equal [503, "maintenance_failed"], [last_response.status, body.dig("error", "code")]
  end

  def test_certificate_names_the_host_the_client_used
    post "/api/v1/maintenance/certificate", JSON.generate(window_id: WINDOW["window_id"], public_key: "ssh-ed25519 AAAA laptop"),
      lan_env.merge("HTTP_X_FORWARDED_HOST" => "evil.example")
    assert_equal 200, last_response.status, last_response.body
    assert_equal({"host" => "stephania-m.local", "port" => 22, "user" => "maint", "principal" => "maint", "serial" => 3}, body.slice("host", "port", "user", "principal", "serial"))
    assert_equal({"window_id" => WINDOW["window_id"], "public_key" => "ssh-ed25519 AAAA laptop", "client_addr" => "192.168.1.20"}, requests.last["params"])
    post "/api/v1/maintenance/certificate", JSON.generate(window_id: "ffffffffffffffff", public_key: "ssh-ed25519 AAAA")
    assert_equal [409, "window_closed"], [last_response.status, body.dig("error", "code")]
  end

  def test_status_close_and_unavailable
    get "/api/v1/maintenance"
    assert_equal 200, last_response.status
    assert_in_delta Time.now.to_i, Time.httpdate(last_response.headers["Date"]).to_i, 2
    assert_equal false, body["open"]
    delete "/api/v1/maintenance/window"
    assert_equal 204, last_response.status
    assert_equal({"actor" => "owner"}, requests.last["params"])
    Control.maint = EngineClient.new(socket_path: File.join(Dir.tmpdir, "stephania-no-maint-#{Process.pid}.sock"))
    get "/api/v1/maintenance"
    assert_equal [503, "maintenance_unavailable"], [last_response.status, body.dig("error", "code")]
  end
end
